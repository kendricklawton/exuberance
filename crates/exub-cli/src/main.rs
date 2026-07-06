//! `exub` — the command-line face of the exuberance engine.
//!
//! Today it runs the cheap-vol screen against a built-in demo universe so you
//! can see the pipeline end-to-end. Once the Polygon data source is wired,
//! `scan` will run against a real symbol list.

use market_data::MockSource;
use signals::{scan, CheapVolCriteria};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");

    match cmd {
        "scan" => run_demo_scan(),
        "version" => println!("exub {}", env!("CARGO_PKG_VERSION")),
        _ => print_help(),
    }
}

fn print_help() {
    println!(
        "exub — exuberance trading engine\n\n\
         USAGE:\n    \
         exub <command>\n\n\
         COMMANDS:\n    \
         scan       Run the cheap-vol screen (demo data until Polygon is wired)\n    \
         version    Print the version\n    \
         help       Show this message\n\n\
         The AI layer lives in Claude Code: try the /vol-scan, /research-ticker,\n\
         and /thesis skills, or hand work to the research-analyst, risk-manager,\n\
         vol-quant, and devils-advocate subagents. See CLAUDE.md."
    );
}

/// Demo scan over a synthetic universe so the pipeline is visible before live
/// data is connected. Replace `MockSource` with `PolygonSource::from_env()`
/// (and a real universe) once the Polygon client is implemented.
fn run_demo_scan() {
    let mut src = MockSource::new();

    // A proven mover trading at cheap implied vol → should surface.
    src.insert_from_closes(
        "MOVER",
        &[100.0, 112.0, 98.5, 110.3, 99.3, 110.2],
        0.22,
        vec![0.18, 0.35, 0.52, 0.60],
    );
    // Cheap IV but barely moves → filtered out.
    src.insert_from_closes(
        "SLEEPY",
        &[100.0, 100.4, 100.1, 100.5, 100.2],
        0.11,
        vec![0.10, 0.45, 0.80],
    );
    // Big mover but IV is expensive for the name → filtered out.
    src.insert_from_closes(
        "PRICEY",
        &[100.0, 112.0, 98.0, 111.0, 99.0],
        0.58,
        vec![0.15, 0.30, 0.60],
    );

    let criteria = CheapVolCriteria {
        lookback_days: 100,
        ..Default::default()
    };
    let universe = ["MOVER", "SLEEPY", "PRICEY"];
    let hits = scan(&src, &universe, &criteria);

    println!("cheap-vol screen — {} candidate(s) from {} scanned\n", hits.len(), universe.len());
    println!(
        "{:<8} {:>6} {:>8} {:>8} {:>8} {:>7}",
        "SYMBOL", "IV", "IVrank", "RVol", "RV/IV", "moves"
    );
    for r in &hits {
        println!(
            "{:<8} {:>5.0}% {:>8} {:>7.0}% {:>8} {:>7}",
            r.symbol,
            r.iv * 100.0,
            r.iv_rank.map(|v| format!("{v:.2}")).unwrap_or_else(|| "-".into()),
            r.realized_vol * 100.0,
            r.realized_over_implied.map(|v| format!("{v:.2}")).unwrap_or_else(|| "-".into()),
            r.big_moves,
        );
    }
    println!("\n(demo data — wire PolygonSource for live results)");
}
