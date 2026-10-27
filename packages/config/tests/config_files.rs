//! Every checked-in environment file must validate against its declared environment.
//!
//! This is the guard `scripts/check-config.sh` runs in CI: broken deployment config
//! failing a production start is excluded by failing the build first. Environment
//! files are thin overlays over `config/example.toml` (see
//! [`Config::from_toml_layers`]), so they are parsed the way the runtime parses them:
//! layered, later document winning.

use lynx_config::schema::{Config, Environment};
use lynx_config::validate::validate;

const EXAMPLE: &str = include_str!("../../../config/example.toml");

fn layered(name: &str, overlay: &'static str) -> Config {
    Config::from_toml_layers(&[EXAMPLE, overlay])
        .unwrap_or_else(|e| panic!("{name} does not parse over the example layer: {e}"))
}

#[test]
fn development_toml_validates_as_development() {
    let config = layered(
        "development.toml",
        include_str!("../../../config/development.toml"),
    );
    let issues = validate(&config).err().unwrap_or_default();
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn production_toml_validates_as_production() {
    let config = layered(
        "production.toml",
        include_str!("../../../config/production.toml"),
    );
    let issues = validate(&config).err().unwrap_or_default();
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn example_toml_validates_for_its_declared_environment() {
    let config = Config::from_toml_str(EXAMPLE).unwrap();
    let issues = validate(&config).err().unwrap_or_default();
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn the_declared_environment_matches_the_file_name() {
    // A file named production.toml whose layer declares "development" is the classic
    // deployment accident: the wrong validator would run at start.
    let dev = layered(
        "development.toml",
        include_str!("../../../config/development.toml"),
    );
    assert_eq!(dev.core.environment, Environment::Development);

    let prod = layered(
        "production.toml",
        include_str!("../../../config/production.toml"),
    );
    assert_eq!(prod.core.environment, Environment::Production);
}
