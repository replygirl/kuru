//! Desktop browser handoff. The browser belongs to the user, not an owned Job.

use std::{io, net::Ipv6Addr, ptr, time::Duration};
use windows_sys::Win32::{
    System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    },
    UI::{
        Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

/// Submit an HTTP(S) URL to the desktop shell. Success acknowledges dispatch,
/// not browser exit or completed navigation. Cancellation or the three-second
/// observation deadline leaves the dedicated desktop thread alone; its owned
/// URL and COM apartment survive until ShellExecuteEx returns.
pub async fn open_http_url(url: &str) -> io::Result<()> {
    open_target(http_url(url)?).await
}

// Only the validated public boundary calls this in production. Native tests
// also use an unregistered fixture scheme, without changing any associations.
async fn open_target(url: Vec<u16>) -> io::Result<()> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("kuru-browser-open".into())
        .spawn(move || {
            let _ = sender.send(handoff(&url));
        })?;
    tokio::time::timeout(Duration::from_secs(3), receiver)
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "desktop browser handoff is still pending",
            )
        })?
        .map_err(|_| io::Error::other("desktop browser handoff worker stopped"))?
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "browser URL requires HTTP(S) and a valid authority",
    )
}

// Deliberately narrow: only a host/optional port and an HTTP(S) URL are accepted,
// never shell namespaces, filesystem paths, credentials or arbitrary protocols.
fn http_url(url: &str) -> io::Result<Vec<u16>> {
    if url.chars().any(|character| {
        character.is_control()
            || character.is_whitespace()
            || matches!(character, '\\' | '"' | '<' | '>')
    }) {
        return Err(invalid());
    }
    let (scheme, rest) = url.split_once("://").ok_or_else(invalid)?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(invalid());
    }
    let authority = rest.split(['/', '?', '#']).next().ok_or_else(invalid)?;
    let (host, port) = if let Some(address) = authority.strip_prefix('[') {
        let (address, suffix) = address.split_once(']').ok_or_else(invalid)?;
        address.parse::<Ipv6Addr>().map_err(|_| invalid())?;
        (
            None,
            if suffix.is_empty() {
                None
            } else {
                Some(suffix.strip_prefix(':').ok_or_else(invalid)?)
            },
        )
    } else {
        let (host, port) = authority
            .split_once(':')
            .map_or((authority, None), |(host, port)| (host, Some(port)));
        (Some(host), port)
    };
    if let Some(host) = host {
        let host = host.strip_suffix('.').unwrap_or(host);
        if host.is_empty()
            || host.len() > 253
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || !label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    || label.starts_with('-')
                    || label.ends_with('-')
            })
        {
            return Err(invalid());
        }
    }
    if let Some(port) = port {
        port.parse::<u16>().map_err(|_| invalid())?;
        if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid());
        }
    }
    let mut wide: Vec<u16> = url.encode_utf16().take(32767).collect();
    if wide.len() == 32767 {
        return Err(invalid());
    }
    wide.push(0);
    Ok(wide)
}

struct Apartment;
impl Apartment {
    fn initialize() -> io::Result<Self> {
        // SAFETY: null reserved pointer and documented STA flags. Production
        // uses a fresh dedicated thread; an incompatible existing apartment is
        // rejected without releasing it. S_OK and S_FALSE both require one
        // matching CoUninitialize on this same thread.
        let result = unsafe {
            CoInitializeEx(
                ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        };
        if result < 0 {
            return Err(io::Error::other(format!(
                "initialize browser COM apartment: HRESULT {result:#x}"
            )));
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: constructed only after successful CoInitializeEx, never moved
        // outside handoff's dedicated thread, exactly one matching release.
        unsafe { CoUninitialize() };
    }
}

fn handoff(url: &[u16]) -> io::Result<()> {
    let _apartment = Apartment::initialize()?;
    let open = [b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0];
    let mut dispatch = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpVerb: open.as_ptr(),
        lpFile: url.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    // SAFETY: owned, NUL-terminated UTF-16 URL and verb outlive this call. The
    // sized structure has null unused fields and no requested process handle.
    // COM remains initialized on this dedicated thread through dispatch:
    // https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw
    // NO_UI suppresses shell error dialogs. NOASYNC requests synchronous launch
    // where supported; Microsoft limits that flag to files, so it is not proof
    // of URI navigation completion. The caller's wait remains independently
    // bounded and never tears down this worker or the user's browser.
    if unsafe { ShellExecuteExW(&mut dispatch) } == 0 {
        // Capture before the COM guard's destructor can change last-error.
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{ERROR_NO_ASSOCIATION, RPC_E_CHANGED_MODE, S_FALSE, S_OK},
        System::Com::COINIT_MULTITHREADED,
    };

    fn unregistered_url() -> Vec<u16> {
        format!("kuru-fixture-{}://handoff", uuid::Uuid::new_v4())
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }

    // Real COM calls must run on one fresh OS thread. A stuck native dispatch
    // fails the bounded observation without synchronously joining that worker.
    async fn on_fresh_thread(operation: impl FnOnce() + Send + 'static) {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation));
            let _ = sender.send(result);
        });
        tokio::time::timeout(Duration::from_secs(3), receiver)
            .await
            .expect("native COM control did not finish within three seconds")
            .expect("native COM control stopped without reporting")
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    }

    struct ComRelease;
    impl Drop for ComRelease {
        fn drop(&mut self) {
            // SAFETY: created only after a successful native initialization,
            // retained and dropped inside the same fresh-thread closure.
            unsafe { CoUninitialize() };
        }
    }

    fn initialize_mta() -> (i32, Option<ComRelease>) {
        // SAFETY: null reserved argument and documented MTA flags. The returned
        // guard balances S_OK and S_FALSE, including assertion unwinding.
        let result = unsafe { CoInitializeEx(ptr::null(), COINIT_MULTITHREADED as u32) };
        (result, (result >= 0).then(|| ComRelease))
    }

    #[tokio::test]
    async fn unregistered_desktop_dispatch_reports_the_actual_native_failure() {
        let error = open_target(unregistered_url()).await.unwrap_err();
        assert_eq!(error.raw_os_error(), Some(ERROR_NO_ASSOCIATION as i32));
    }

    #[tokio::test]
    async fn failed_desktop_dispatch_releases_its_sta_apartment() {
        on_fresh_thread(|| {
            let error = handoff(&unregistered_url()).unwrap_err();
            assert_eq!(error.raw_os_error(), Some(ERROR_NO_ASSOCIATION as i32));
            let (result, _release) = initialize_mta();
            // A leaked STA reference would make the actual mode change fail.
            assert_eq!(result, S_OK);
        })
        .await;
    }

    #[tokio::test]
    async fn rejected_apartment_change_preserves_the_callers_mta() {
        on_fresh_thread(|| {
            let (initial, _first_release) = initialize_mta();
            assert_eq!(initial, S_OK);
            let error = handoff(&unregistered_url()).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("initialize browser COM apartment: HRESULT {RPC_E_CHANGED_MODE:#x}")
            );
            let (retained, _second_release) = initialize_mta();
            // Incorrectly uninitializing the rejected STA call would instead
            // remove our original MTA reference and make this return S_OK.
            assert_eq!(retained, S_FALSE);
        })
        .await;
    }

    #[tokio::test]
    async fn invalid_urls_fail_before_any_desktop_handoff() {
        for url in [
            "",
            "file:///tmp/login",
            "ms-settings:privacy",
            "https:///missing",
            "https://user:password@example.com",
            "https://example.com\\other",
            "https://host:65536/",
            "https://host:/",
            "https://host:+443/",
            "https://[invalid]/",
            "https://[::1]suffix/",
            "https://-host/",
            "https://a..b/",
            "https://host/\0secret",
            "https://host/\nsecret",
            "https://host/with space",
            "https://host/\"quoted\"",
            "https://host/<file>",
        ] {
            let error = open_http_url(url).await.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(!error.to_string().contains("secret"));
        }
        assert!(http_url(&format!("https://host/{}", "x".repeat(32767))).is_err());
    }

    #[test]
    fn accepted_http_urls_retain_exact_utf16_without_opening_a_browser() {
        for url in [
            "https://auth.openai.com/oauth/authorize?state=a&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback",
            "HTTP://localhost:1455/auth/callback",
            "https://[::1]:443/",
            "https://example.com/日本語/🦀",
        ] {
            let wide = http_url(url).unwrap();
            assert_eq!(wide.last(), Some(&0));
            assert_eq!(String::from_utf16(&wide[..wide.len() - 1]).unwrap(), url);
        }
    }
}
