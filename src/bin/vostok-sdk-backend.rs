use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use vostok_resource_porter::sdk::{DiagnosticSeverity, SdkEvent, SdkRequest};

#[derive(Parser)]
#[command(version, about = "Backend process for the recovered Vostok SDK")]
struct Args {
    /// JSON request file. Reads one request from stdin when omitted.
    #[arg(long)]
    request: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let bytes = match args.request {
        Some(path) => {
            fs::read(&path).with_context(|| format!("reading SDK request {}", path.display()))?
        }
        None => {
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes)?;
            bytes
        }
    };
    let request: SdkRequest = serde_json::from_slice(&bytes).context("parsing SDK request")?;
    let result = vostok_resource_porter::sdk::execute(request, print_event);
    if let Err(error) = result {
        print_event(SdkEvent::Diagnostic {
            severity: DiagnosticSeverity::Error,
            code: "sdk.operation.failed".into(),
            message: format!("{error:#}"),
            resource: None,
        });
        print_event(SdkEvent::Complete {
            success: false,
            summary: serde_json::json!({"error": format!("{error:#}")}),
        });
        return Err(error);
    }
    Ok(())
}

fn print_event(event: SdkEvent) {
    println!(
        "{}",
        serde_json::to_string(&event).expect("serializing SDK event")
    );
}
