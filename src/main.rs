use anyhow::Result;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use remote_zip_peek::{RemoteHttpReader, ZipExplorer};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// URL of the ZIP file to explore
    url: String,

    /// Display sizes in human-readable format (e.g., KB, MB)
    #[arg(short = 'H', long)]
    human_readable: bool,
}

fn format_size(bytes: u64, human: bool) -> String {
    if !human {
        return format!("{}", bytes);
    }

    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    format!("{:.2} {}", size, UNITS[unit_index])
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("Fetching ZIP from: {}", args.url);

    // Initialize Progress Bar
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} {bytes}")
            .unwrap()
            .tick_chars("/|\\- "),
    );
    pb.set_message("Fetching metadata...");
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    let reader = RemoteHttpReader::new(&args.url, Some(pb.clone()))?;
    let mut explorer = ZipExplorer::new(reader);

    let files = explorer.list_files()?;

    // Finish progress bar
    pb.finish_and_clear();

    let total_size = explorer.get_total_size();
    let fetched = explorer.get_bytes_fetched();

    println!("Total file size: {} bytes", total_size);
    println!("\nFound {} files:", files.len());

    for (name, size, compressed) in files {
        println!(
            "- {} (Size: {}, Compressed: {})",
            name,
            format_size(size, args.human_readable),
            format_size(compressed, args.human_readable)
        );
    }

    println!("\nStats:");
    println!(
        "Total file size: {}",
        format_size(total_size, args.human_readable)
    );
    println!(
        "Data fetched: {}",
        format_size(fetched, args.human_readable)
    );
    if total_size > 0 {
        println!(
            "Efficiency: {:.2}% fetched",
            (fetched as f64 / total_size as f64) * 100.0
        );
    }

    Ok(())
}
