//! Resolving the content-hashed stylesheet URL.
//!
//! Vite emits `public.<hash>.css` plus a manifest mapping the source path to it.
//! Reading that manifest **at startup** — rather than `include_str!`ing the CSS
//! into the binary — is what keeps `cargo build` independent of whether pnpm has
//! run. The cost is one file read per boot.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

/// The entry key Vite uses, which is the input path from vite.public.config.ts.
const ENTRY: &str = "src/public.css";

#[derive(Debug, Clone)]
pub struct Assets {
    /// Absolute path from the site root, e.g. `/assets/public-DMw5U-C1.css`.
    pub css: String,
}

#[derive(Deserialize)]
struct ManifestEntry {
    file: String,
}

impl Assets {
    pub fn load(dist_dir: &Path) -> Result<Self> {
        let manifest_path = dist_dir.join(".vite").join("manifest.json");

        let raw = std::fs::read_to_string(&manifest_path).with_context(|| {
            format!(
                "reading {}. Run `pnpm --dir web run build:public` first — the public site \
                 cannot render without its stylesheet.",
                manifest_path.display()
            )
        })?;

        let manifest: std::collections::HashMap<String, ManifestEntry> =
            serde_json::from_str(&raw)
                .with_context(|| format!("parsing {}", manifest_path.display()))?;

        let Some(entry) = manifest.get(ENTRY) else {
            bail!(
                "{} has no entry for {ENTRY}. Did vite.public.config.ts change its input?",
                manifest_path.display()
            );
        };

        Ok(Self {
            css: format!("/{}", entry.file),
        })
    }
}
