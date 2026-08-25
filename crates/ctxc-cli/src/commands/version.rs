//! `ctxc --version`

use std::io::{self, Write};

use anyhow::Result;
use serde::Serialize;

use ctxc_core::platform::Os;

use crate::output::{Printer, Render};

/// Build identity of this binary.
#[derive(Debug, Serialize)]
pub struct VersionReport {
    pub name: &'static str,
    pub version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    /// Schema version this build migrates databases to.
    pub schema_version: u32,
}

impl VersionReport {
    pub fn collect() -> Self {
        VersionReport {
            name: "ctxc",
            version: env!("CARGO_PKG_VERSION"),
            os: Os::current().as_str(),
            arch: std::env::consts::ARCH,
            schema_version: ctxc_store::latest_schema_version(),
        }
    }
}

impl Render for VersionReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out, "{} {}", self.name, self.version)?;
        writeln!(out, "platform: {} ({})", self.os, self.arch)?;
        writeln!(out, "schema:   {}", self.schema_version)
    }
}

pub fn run<W: Write>(printer: &mut Printer<W>) -> Result<()> {
    printer.emit(&VersionReport::collect())?;
    Ok(())
}
