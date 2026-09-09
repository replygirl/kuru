#![cfg(feature = "tooling")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use kuru_delivery::docs;

struct Site {
    directory: tempfile::TempDir,
    root: PathBuf,
}

impl Site {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("dist");
        let site = Self { directory, root };
        site.write("index.html", "<h1 id=\"kuru\">Kuru</h1>");
        site.write("sitemap.xml", "<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>https://replygirl.github.io/kuru/</loc></url></urlset>");
        site.write("llms.txt", "# Kuru\nPublic guide to Kuru.\n");
        site.write("llms-full.txt", "# Kuru\nComplete public guide to Kuru.\n");
        site
    }

    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    fn errors(&self) -> Vec<String> {
        docs::check(&self.root, "/kuru/").unwrap()
    }

    fn cli(&self, base: &str) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_kuru-delivery"))
            .args(["docs", "--root"])
            .arg(&self.root)
            .args(["--base", base])
            .output()
            .unwrap()
    }
}

#[test]
fn real_pages_resolve_clean_urls_resources_unicode_and_destination_anchors() {
    let site = Site::new();
    site.write("index.html", "<h1 id=\"kuru\">Kuru</h1><a href=\"#kuru\">Home</a><a href=\"/kuru/guide/start#install\">Install</a><a href=\"guide/\">Guide</a><script src=\"/kuru/assets/app.js?v=1\"></script>");
    site.write("guide/start.html", "<h1 id=\"install\">Install</h1><a name=\"legacy\"></a><a href=\"../index.html#kuru\">Home</a><a href=\"?mode=light#legacy\">Old anchor</a><img src=\"../assets/logo.svg\"><h2 id=\"café\">Café</h2><a href=\"#caf%C3%A9\">Unicode</a><a href=\"#install:~:text=Install\">Text fragment</a>");
    site.write("guide/index.html", "<a href=\"start#install\">Start</a>");
    site.write("assets/app.js", "document.title = 'Kuru';");
    site.write(
        "assets/logo.svg",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
    );
    assert_eq!(site.errors(), Vec::<String>::new());
}

#[test]
fn external_data_and_mail_links_do_not_require_network() {
    let site = Site::new();
    site.write("index.html", "<a href=\"https://example.invalid/unreachable#absent\">External</a><a href=\"//example.invalid/remote\">Remote</a><a href=\"mailto:hello@example.invalid\">Mail</a><img src=\"data:image/svg+xml,%3Csvg%3E%3C/svg%3E\">");
    assert!(site.errors().is_empty());
}

#[test]
fn missing_scripts_images_and_documents_each_fail() {
    let site = Site::new();
    site.write("index.html", "<script src=\"/kuru/assets/missing.js\"></script><img src=\"missing.svg\"><a href=\"missing-page\">Missing</a>");
    let errors = site.errors();
    assert_eq!(errors.len(), 3);
    assert!(
        errors
            .iter()
            .all(|error| error.contains("missing local target"))
    );
}

#[test]
fn anchor_must_exist_in_its_destination_document() {
    let site = Site::new();
    site.write(
        "index.html",
        "<h1 id=\"install\">Home</h1><a href=\"guide.html#install\">Guide</a>",
    );
    site.write("guide.html", "<h1 id=\"guide\">Guide</h1>");
    assert_eq!(site.errors().len(), 1);
    assert!(site.errors()[0].contains("missing HTML anchor"));
}

#[test]
fn incorrect_base_and_encoded_traversal_are_rejected() {
    let site = Site::new();
    for link in [
        "/assets/app.js",
        "/kuru-other/index.html",
        "../index.html",
        "/kuru/%2e%2e/index.html",
        "/kuru/guide/../../index.html",
        "/kuru/%2e%2e%2findex.html",
    ] {
        site.write("index.html", &format!("<a href=\"{link}\">Link</a>"));
        let errors = site.errors();
        assert_eq!(errors.len(), 1, "{link}: {errors:?}");
        assert!(errors[0].contains("escapes site base"), "{errors:?}");
    }
}

#[cfg(unix)]
#[test]
fn output_symlinks_cannot_publish_outside_files_or_directories() {
    use std::os::unix::fs::symlink;
    let site = Site::new();
    let outside = site.directory.path().join("outside.html");
    fs::write(&outside, "outside").unwrap();
    symlink(&outside, site.root.join("outside.html")).unwrap();
    site.write("index.html", "<a href=\"outside.html\">Outside</a>");
    let errors = site.errors();
    assert_eq!(errors.len(), 2);
    assert!(
        errors
            .iter()
            .all(|error| error.contains("escapes output directory"))
    );
    fs::remove_file(site.root.join("outside.html")).unwrap();
    symlink(
        site.directory.path().join("missing"),
        site.root.join("alias"),
    )
    .unwrap();
    site.write(
        "index.html",
        "<a href=\"alias/secret.html\">Missing outside</a>",
    );
    assert!(
        site.errors()
            .iter()
            .all(|error| error.contains("escapes output directory"))
    );
}

#[test]
fn file_urls_cannot_reference_host_files() {
    let site = Site::new();
    let outside = site.directory.path().join("outside.html");
    fs::write(&outside, "outside").unwrap();
    let url = url::Url::from_file_path(outside).unwrap();
    site.write("index.html", &format!("<a href=\"{url}\">Host file</a>"));
    assert_eq!(site.errors().len(), 1);
    assert!(site.errors()[0].contains("escapes output directory"));
}

#[test]
fn required_build_artifacts_must_be_nonempty() {
    let site = Site::new();
    for name in ["index.html", "sitemap.xml", "llms.txt", "llms-full.txt"] {
        let path = site.root.join(name);
        let original = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(
            site.errors()
                .iter()
                .any(|error| error.contains(name) && error.contains("missing or empty"))
        );
        fs::write(&path, []).unwrap();
        assert!(
            site.errors()
                .iter()
                .any(|error| error.contains(name) && error.contains("missing or empty"))
        );
        fs::write(path, original).unwrap();
    }
}

#[test]
fn repository_only_pages_and_private_artifacts_fail() {
    let site = Site::new();
    for name in [
        "verification.html",
        "reference/implementation-contract.html",
        "assets/verification.md.abcdef.js",
        "AGENTS.md",
        "openspec/changes/private/proposal.md",
        ".env.fixture",
        "state/memory.sqlite3-wal",
        ".codex/auth.json",
    ] {
        let path = site.write(name, "fixture only");
        assert!(
            site.errors()
                .contains(&format!("{name}: repository-only or private artifact"))
        );
        fs::remove_file(path).unwrap();
    }
    assert!(site.errors().iter().any(|error| error.contains("openspec")));
}

#[test]
fn sitemap_destinations_obey_base_exist_and_use_valid_xml() {
    let site = Site::new();
    site.write("sitemap.xml", "<urlset><url><loc>https://docs.invalid/missing-base/</loc></url><url><loc>https://docs.invalid/kuru/missing</loc></url></urlset>");
    let errors = site.errors();
    assert_eq!(errors.len(), 2);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("escapes site base"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("missing local target"))
    );
    site.write("sitemap.xml", "not XML");
    assert!(site.errors()[0].contains("invalid XML"));
    site.write("sitemap.xml", "<urlset/>");
    assert!(site.errors()[0].contains("no page locations"));
}

#[test]
fn custom_base_and_real_cli_exit_status() {
    let site = Site::new();
    site.write(
        "index.html",
        "<a href=\"/other/#home\">Home</a><h1 id=\"home\">Home</h1>",
    );
    site.write(
        "sitemap.xml",
        "<urlset><url><loc>https://docs.invalid/other/</loc></url></urlset>",
    );
    let output = site.cli("/other/");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    site.write("index.html", "<img src=\"missing.png\">");
    let output = site.cli("/other/");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing.png"));
}

#[test]
fn invalid_bases_bad_encoding_and_unreadable_html_fail_locally() {
    let site = Site::new();
    for base in [
        "kuru/", "/kuru", "/../", "/a//b/", "/a?b/", "/%61/", "/a\\b/", "/a\n/",
    ] {
        assert!(docs::check(&site.root, base).unwrap()[0].contains("base must"));
    }
    assert!(
        docs::check(Path::new("/nonexistent/kuru-docs-fixture"), "/").unwrap()[0]
            .contains("does not exist")
    );
    for link in [
        "/kuru/%00",
        "/kuru/%5c",
        "/kuru/%ZZ",
        "/kuru/%FF",
        "/kuru/#%",
        "/kuru/%Q1",
    ] {
        site.write("index.html", &format!("<a href=\"{link}\">Broken</a>"));
        assert!(
            site.errors()
                .iter()
                .any(|error| error.contains("invalid link")),
            "{link}: {:?}",
            site.errors()
        );
    }
    site.write("index.html", "<h1>Kuru</h1>");
    fs::write(site.root.join("broken.html"), [0xff]).unwrap();
    assert!(
        site.errors()
            .iter()
            .any(|error| error.contains("cannot read HTML"))
    );
}

#[cfg(unix)]
#[test]
fn internal_symlinks_and_root_base_work_without_recursing_cycles() {
    use std::os::unix::fs::symlink;
    let site = Site::new();
    site.write(
        "index.html",
        "<h1 id=\"home\">Kuru</h1><a href=\"/alias.html#home\">Alias</a>",
    );
    site.write(
        "sitemap.xml",
        "<urlset><url><loc>https://docs.invalid/</loc></url></urlset>",
    );
    symlink("index.html", site.root.join("alias.html")).unwrap();
    symlink(".", site.root.join("cycle")).unwrap();
    assert!(docs::check(&site.root, "/").unwrap().is_empty());
}
