//! Desktop browser handoff. The browser belongs to the user, not an owned Job.

use std::{io, net::Ipv6Addr, ptr, time::Duration};
use windows_sys::Win32::{
    System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    },
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};

/// Submit an HTTP(S) URL to the desktop shell. Success acknowledges dispatch,
/// not browser exit or completed navigation. Cancellation or the three-second
/// observation deadline leaves the dedicated desktop thread alone; its owned
/// URL and COM apartment survive until ShellExecute returns.
pub async fn open_http_url(url: &str) -> io::Result<()> {
    let url = http_url(url)?;
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
        // SAFETY: this dedicated thread has no ambient apartment. Null reserved
        // pointer and documented STA flags; S_OK and S_FALSE both require one
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
    // SAFETY: owned, validated, NUL-terminated UTF-16 URL and verb outlive this
    // synchronous call. No window, parameters or working directory are supplied.
    // COM is initialized as advised by the primary API contract:
    // https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecutew
    // The return is a historical integer result, NOT an owned HINSTANCE/handle.
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            open.as_ptr(),
            url.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        return Err(io::Error::other(format!(
            "desktop browser handoff failed (shell code {result})"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
