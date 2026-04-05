use crate::cli::InitArgs;
use crate::templates;
use anyhow::{Context, Result, anyhow};
use std::fs;
use std::path::Path;

pub(crate) fn run_init(args: InitArgs) -> Result<()> {
    if args.list {
        for template in templates::all() {
            println!("{}\t{}", template.id, template.description);
        }
        return Ok(());
    }

    let template_id = if let Some(template) = &args.template {
        template.clone()
    } else {
        let db_type = args
            .db_type
            .as_deref()
            .context("--template or the combination of --db-type and --mode is required")?;
        let mode = args
            .mode
            .as_deref()
            .context("--template or the combination of --db-type and --mode is required")?;
        templates::resolve_shortcut(db_type, mode)
            .ok_or_else(|| anyhow!("unsupported template shortcut: {} + {}", db_type, mode))?
            .to_string()
    };

    let template =
        templates::get(&template_id).ok_or_else(|| anyhow!("unknown template: {}", template_id))?;
    let output = args
        .output
        .as_deref()
        .context("--output is required unless --list is used")?;
    let output_path = Path::new(output);

    if output_path.exists() && !args.force {
        return Err(anyhow!(
            "output file already exists: {} (use --force to overwrite)",
            output
        ));
    }

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    fs::write(output_path, template.content)?;
    println!("Wrote template '{}' to {}", template.id, output);
    Ok(())
}
