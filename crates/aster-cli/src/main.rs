use std::collections::HashSet;
use std::path::{Path, PathBuf};

use aster_analysis::{
    AnalysisOptions, AnalysisResult, Diagnostic, LookupExplanation, LookupResult, ModuleGraph,
    analyze, explain,
};
use clap::{Parser, Subcommand};
use serde::Serialize;

#[derive(Parser)]
#[command(name = "aster", version, about = "Lua-first IDE tooling")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print the require/module graph of a Lua project.
    Graph {
        /// Project root directory.
        dir: PathBuf,
        /// Emit JSON instead of a text tree.
        #[arg(long)]
        json: bool,
    },
    /// Run Lua-specific checks (sequence diagnostics, multiple returns).
    Check {
        /// Project root directory.
        dir: PathBuf,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Explain how member accesses on a source line resolve through metatables.
    Explain {
        /// Project root directory.
        dir: PathBuf,
        /// Lua file relative to the project root.
        file: PathBuf,
        /// 1-based source line.
        line: usize,
    },
}

#[derive(Serialize)]
struct JsonOutput<'a> {
    modules: Vec<JsonModule>,
    diagnostics: &'a [Diagnostic],
}

#[derive(Serialize)]
struct CheckJsonOutput<'a> {
    diagnostics: &'a [Diagnostic],
}

#[derive(Serialize)]
struct JsonModule {
    path: PathBuf,
    dependencies: Vec<PathBuf>,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Graph { dir, json } => {
            if !dir.is_dir() {
                eprintln!("error: '{}' is not a readable directory", dir.display());
                std::process::exit(1);
            }
            let result = analyze(&AnalysisOptions::new(&dir));
            if json {
                print_json(&result);
            } else {
                print!("{}", render_tree(&result));
                print_diagnostics(&result.diagnostics);
            }
        }
        Commands::Check { dir, json } => {
            if !dir.is_dir() {
                eprintln!("error: '{}' is not a readable directory", dir.display());
                std::process::exit(1);
            }
            let diagnostics = aster_analysis::check(&AnalysisOptions::new(&dir));
            if json {
                let output = CheckJsonOutput {
                    diagnostics: &diagnostics,
                };
                println!("{}", serde_json::to_string_pretty(&output).unwrap());
            } else if diagnostics.is_empty() {
                println!("No issues found.");
            } else {
                print_diagnostics(&diagnostics);
            }
        }
        Commands::Explain { dir, file, line } => {
            if !dir.is_dir() {
                eprintln!("error: '{}' is not a readable directory", dir.display());
                std::process::exit(1);
            }
            let explanations = explain(&AnalysisOptions::new(&dir), &file, line);
            if explanations.is_empty() {
                println!("No known member accesses on line {line}.");
            } else {
                print_explanations(&explanations);
            }
        }
    }
}

fn print_json(result: &AnalysisResult) {
    let modules = result.graph.modules();
    let json_modules: Vec<JsonModule> = modules
        .iter()
        .map(|module| JsonModule {
            path: module.to_path_buf(),
            dependencies: result
                .graph
                .dependencies(module)
                .iter()
                .map(|d| d.to_path_buf())
                .collect(),
        })
        .collect();
    let output = JsonOutput {
        modules: json_modules,
        diagnostics: &result.diagnostics,
    };
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

/// Render the module tree from each entry point, `├──`/`└──` style.
/// Already-expanded modules are not re-expanded; cycles are marked.
fn render_tree(result: &AnalysisResult) -> String {
    let mut out = String::new();
    let roots = result.graph.entry_points();
    let mut expanded = HashSet::new();
    let mut stack = Vec::new();
    for (i, root) in roots.iter().enumerate() {
        render_node(
            &result.graph,
            root,
            "",
            i + 1 == roots.len(),
            &mut expanded,
            &mut stack,
            &mut out,
        );
    }
    out
}

fn render_node(
    graph: &ModuleGraph,
    path: &Path,
    prefix: &str,
    is_last: bool,
    expanded: &mut HashSet<PathBuf>,
    stack: &mut Vec<PathBuf>,
    out: &mut String,
) {
    let connector = if is_last { "└── " } else { "├── " };
    if stack.iter().any(|p| p == path) {
        out.push_str(&format!("{prefix}{connector}{} (cycle)\n", path.display()));
        return;
    }
    out.push_str(&format!("{prefix}{connector}{}\n", path.display()));
    if !expanded.insert(path.to_path_buf()) {
        return; // already expanded elsewhere in the tree
    }
    stack.push(path.to_path_buf());
    let child_prefix = format!("{prefix}{}", if is_last { "    " } else { "│   " });
    let deps = graph.dependencies(path);
    for (i, dep) in deps.iter().enumerate() {
        render_node(
            graph,
            dep,
            &child_prefix,
            i + 1 == deps.len(),
            expanded,
            stack,
            out,
        );
    }
    stack.pop();
}

fn print_diagnostics(diagnostics: &[Diagnostic]) {
    if diagnostics.is_empty() {
        return;
    }
    println!("\nDiagnostics:");
    for d in diagnostics {
        let location = match (&d.file, d.line, d.column) {
            (Some(file), Some(line), Some(column)) => {
                format!("{}:{}:{}: ", file.display(), line, column)
            }
            (Some(file), _, _) => format!("{}: ", file.display()),
            _ => String::new(),
        };
        println!("  [{:?}] {}{}", d.kind, location, d.message);
    }
}

fn print_explanations(explanations: &[LookupExplanation]) {
    for explanation in explanations {
        println!(
            "{} ({}:{})",
            explanation.expression, explanation.line, explanation.column
        );
        for step in &explanation.steps {
            println!("  ↓ {step}");
        }
        match &explanation.result {
            LookupResult::Found(message) => println!("  = {message}"),
            LookupResult::NotFound => println!("  = member not found"),
            LookupResult::Unknown(reason) => println!("  = unknown: {reason}"),
        }
    }
}
