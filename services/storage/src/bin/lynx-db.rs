//! `lynx-db` — the operator entry point for the operational schema.
//!
//! Thin CLI over [`lynx_storage`] so the shell scripts in `scripts/` (and therefore
//! the `make migrate*` targets) drive the same ledger and the same checksums the
//! services use at startup. There is deliberately no second migration tool: the
//! `schema_migrations` ledger in `database/migrations/README.md` has exactly one
//! writer.
//!
//! ```text
//! lynx-db up      [migrations-dir]   apply pending migrations
//! lynx-db status  [migrations-dir]   list applied and pending
//! lynx-db seed    [seeds-dir]        run every *.sql in the seed directory
//! lynx-db new     NAME               scaffold the next NNNN_NAME.sql
//! lynx-db retention-check            verify retention_policy covers every timed table
//! lynx-db sweep                      one retention sweep pass (§Z Step 17)
//! ```

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lynx_storage::{migrations, Storage};

/// Default migration directory, resolved at compile time so the tool works
/// regardless of the working directory.
fn default_migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../database/migrations")
}

/// Default seed directory, resolved at compile time.
fn default_seeds_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../database/seeds")
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map_or("help", String::as_str);

    match run(command, &args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lynx-db: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(command: &str, args: &[String]) -> Result<(), Box<dyn Error>> {
    let override_dir = args.get(1).map(PathBuf::from);

    match command {
        "up" => {
            let dir = override_dir.unwrap_or_else(default_migrations_dir);
            let storage = connect().await?;
            let applied = storage.migrate(&dir).await?;
            if applied.is_empty() {
                println!("up to date (0 pending)");
            } else {
                for version in &applied {
                    println!("applied {version:04}");
                }
            }
            storage.close().await;
        }
        "status" => {
            let dir = override_dir.unwrap_or_else(default_migrations_dir);
            let storage = connect().await?;
            let applied = migrations::load_applied(storage.pool()).await?;
            let pending = storage.pending_migrations(&dir).await?;

            println!("applied: {}", applied.len());
            for migration in &applied {
                println!("  {:04}  {}", migration.version, migration.name);
            }
            println!("pending: {}", pending.len());
            for migration in &pending {
                println!("  {:04}  {}", migration.version, migration.name);
            }
            storage.close().await;
        }
        "seed" => {
            let dir = override_dir.unwrap_or_else(default_seeds_dir);
            let storage = connect().await?;
            seed(storage.pool(), &dir).await?;
            storage.close().await;
        }
        "new" => {
            let name = args.get(1).ok_or("usage: lynx-db new <name>")?.as_str();
            let dir = args
                .get(2)
                .map_or_else(default_migrations_dir, PathBuf::from);
            scaffold(&dir, name)?;
        }
        "retention-check" => {
            let storage = connect().await?;
            let violations = lynx_storage::retention::verify_policy(storage.pool()).await?;
            if violations.is_empty() {
                println!("retention policy covers every timed table");
            } else {
                for violation in &violations {
                    eprintln!("violation: {violation}");
                }
                return Err(format!("{} retention policy violation(s)", violations.len()).into());
            }
            storage.close().await;
        }
        "sweep" => {
            let storage = connect().await?;
            let violations = lynx_storage::retention::verify_policy(storage.pool()).await?;
            if !violations.is_empty() {
                for violation in &violations {
                    eprintln!("violation: {violation}");
                }
                return Err(format!(
                    "refusing to sweep with {} policy violation(s): a half-wired \
                     retention story is worse than a loudly absent one",
                    violations.len()
                )
                .into());
            }
            let results = lynx_storage::retention::sweep(storage.pool()).await?;
            if results.is_empty() {
                println!("nothing swept (no class had a bounded policy)");
            } else {
                for result in &results {
                    println!(
                        "{:>18}  {:<18} {} row(s) deleted",
                        result.class, result.table, result.deleted
                    );
                }
            }
            storage.close().await;
        }
        "help" | "--help" | "-h" => print_help(),
        other => {
            eprintln!("lynx-db: unknown command `{other}`");
            print_help();
            return Err("no such command".into());
        }
    }
    Ok(())
}

async fn connect() -> Result<Storage, Box<dyn Error>> {
    let url =
        std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is not set (see .env.example)")?;
    Ok(Storage::connect(&url, 5).await?)
}

async fn seed(pool: &sqlx::PgPool, dir: &Path) -> Result<(), Box<dyn Error>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) == Some("sql") {
            files.push(path);
        }
    }
    files.sort();

    if files.is_empty() {
        return Err(format!("no *.sql seeds found in {}", dir.display()).into());
    }

    for path in files {
        let sql = std::fs::read_to_string(&path)?;
        sqlx::raw_sql(&sql).execute(pool).await?;
        println!("seeded {}", path.display());
    }
    Ok(())
}

/// Create the next migration file with the required header.
fn scaffold(dir: &Path, name: &str) -> Result<(), Box<dyn Error>> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err("name must be lower_snake_case (a-z, 0-9, _)".into());
    }

    let next = migrations::discover(dir)?
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap_or(0)
        + 1;
    let file = dir.join(format!("{next:04}_{name}.sql"));
    if file.exists() {
        return Err(format!("{} already exists", file.display()).into());
    }

    let body = format!(
        "-- {next:04}_{name}.sql\n\
         -- +migrate:expand\n\
         --\n\
         -- Purpose : \n\
         -- Duration: \n\
         -- Plans   : \n\
         -- Indexes : \n"
    );
    std::fs::write(&file, body)?;
    println!("created {}", file.display());
    Ok(())
}

fn print_help() {
    println!(
        "lynx-db — LYNX operational schema tool\n\n\
         USAGE:\n  \
         lynx-db up      [migrations-dir]   apply pending migrations\n  \
         lynx-db status  [migrations-dir]   list applied and pending\n  \
         lynx-db seed    [seeds-dir]        run every *.sql in the seed directory\n  \
         lynx-db new     NAME               scaffold the next NNNN_NAME.sql\n  \
         lynx-db retention-check            verify retention_policy covers every timed table\n  \
         lynx-db sweep                      one retention sweep pass (cron-style job)\n\n\
         DATABASE_URL must be set for up/status/seed/retention-check/sweep. There is\n\
         no `down`: migrations are forward-only; roll back the application, not the\n\
         schema."
    );
}
