//! Prints the feature registry as a Markdown table (used to generate docs/CONFIGURATION.md).
use aqua_glass::features::{Category, Preset, FEATURES};

fn main() {
    for c in Category::ALL {
        let fs: Vec<_> = FEATURES.iter().filter(|f| f.category == c).collect();
        if fs.is_empty() {
            continue;
        }
        println!("\n### {}\n", c.label());
        println!("| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |");
        println!("|---|---|---|---|---|---|---|---|---|---|");
        for f in fs {
            let b = |p: Preset| if f.preset_value(p).unwrap_or(false) { "✔" } else { "" };
            println!(
                "| `{}` | {} | {} | +{}–{} MB | {} | {} | {} | {} | {} | {} |",
                f.id,
                f.name,
                f.impact.label(),
                f.est_mb.0,
                f.est_mb.1,
                f.deps.iter().map(|d| format!("`{d}`")).collect::<Vec<_>>().join(", "),
                if f.visual { "✔" } else { "" },
                b(Preset::Minimal),
                b(Preset::Balanced),
                b(Preset::Enhanced),
                b(Preset::FullyCustomized)
            );
        }
    }
}
