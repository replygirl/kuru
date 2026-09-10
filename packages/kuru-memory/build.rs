#[path = "support/bundle.rs"]
mod bundle;

use anyhow::{Context, Result};
use std::{env, fs, path::PathBuf};

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=support/bundle.rs");
    println!("cargo:rerun-if-changed=support/dolt-assets.json");
    println!("cargo:rerun-if-env-changed=KURU_DOLT_BUNDLE_DIR");
    let package =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").context("Cargo package directory")?);
    let target = env::var("TARGET").context("Cargo TARGET is required; no host fallback")?;
    let prepare = format!(
        "prepare the matching bundled runtime with `mise run //packages/kuru-memory:bundle:prepare -- --target {target}`; local archive import and offline mirrors are supported"
    );
    let manifest = bundle::Manifest::load(&package.join("support/dolt-assets.json"))?;
    let asset = manifest.select(&target).with_context(|| prepare.clone())?;
    let override_path = env::var_os("KURU_DOLT_BUNDLE_DIR").map(PathBuf::from);
    let directory = bundle::bundle_directory(&package, override_path.as_deref())?;
    let archive = bundle::prepared_archive(&directory, asset);
    println!("cargo:rerun-if-changed={}", archive.display());
    let bytes = bundle::verified_archive(&archive, asset)
        .with_context(|| format!("{}; {prepare}", archive.display()))?;
    let out = PathBuf::from(env::var_os("OUT_DIR").context("Cargo OUT_DIR")?);
    fs::write(out.join("dolt.archive"), bytes)
        .context("copy verified bundled Dolt into Cargo output")?;
    fs::write(out.join("dolt_catalog.rs"), manifest.catalog(&target)?)?;
    Ok(())
}
