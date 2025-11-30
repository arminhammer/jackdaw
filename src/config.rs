use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Global configuration for Jackdaw
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JackdawConfig {
    #[serde(default)]
    pub run: RunConfig,
    #[serde(default)]
    pub validate: ValidateConfig,
    #[serde(default)]
    pub visualize: VisualizeConfig,
}

/// Configuration for the 'run' command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    /// Path to the durable persistence database
    pub durable_db: Option<PathBuf>,

    /// Path to the cache database (if different from durable db)
    pub cache_db: Option<PathBuf>,

    /// Run workflows in parallel
    #[serde(default)]
    pub parallel: bool,

    /// Enable verbose output
    #[serde(default)]
    pub verbose: bool,

    /// Skip cache hits (force re-execution)
    #[serde(default)]
    pub no_cache: bool,

    /// Generate workflow visualization after execution
    #[serde(default)]
    pub visualize: bool,

    /// Visualization tool to use (graphviz or d2)
    pub viz_tool: Option<String>,

    /// Visualization output format (svg, png, pdf, ascii)
    pub viz_format: Option<String>,

    /// Visualization output path
    pub viz_output: Option<PathBuf>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            durable_db: Some(PathBuf::from("workflow.db")),
            cache_db: None,
            parallel: false,
            verbose: false,
            no_cache: false,
            visualize: false,
            viz_tool: Some("d2".to_string()),
            viz_format: Some("svg".to_string()),
            viz_output: None,
        }
    }
}

/// Configuration for the 'validate' command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateConfig {
    /// Show verbose output including all expressions checked
    #[serde(default)]
    pub verbose: bool,
}

impl Default for ValidateConfig {
    fn default() -> Self {
        Self {
            verbose: false,
        }
    }
}

/// Configuration for the 'visualize' command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualizeConfig {
    /// Path to the durable persistence database
    pub durable_db: Option<PathBuf>,

    /// Visualization tool to use (graphviz or d2)
    pub tool: Option<String>,

    /// Output format (svg, png, pdf, ascii)
    pub format: Option<String>,
}

impl Default for VisualizeConfig {
    fn default() -> Self {
        Self {
            durable_db: Some(PathBuf::from("workflow.db")),
            tool: Some("graphviz".to_string()),
            format: Some("svg".to_string()),
        }
    }
}

impl JackdawConfig {
    /// Load configuration from multiple sources with precedence:
    /// 1. Command line arguments (highest priority)
    /// 2. Environment variables (JACKDAW_*)
    /// 3. Config file (jackdaw.yaml in current dir or ~/.config/jackdaw/jackdaw.yaml)
    /// 4. Defaults (lowest priority)
    pub fn load() -> Result<Self, config::ConfigError> {
        let config_builder = config::Config::builder()
            // Start with defaults
            .add_source(config::Config::try_from(&JackdawConfig::default())?)
            // Add config file from current directory
            .add_source(
                config::File::with_name("jackdaw")
                    .format(config::FileFormat::Yaml)
                    .required(false),
            )
            // Add config file from user's config directory
            .add_source(
                config::File::with_name(&format!(
                    "{}/.config/jackdaw/jackdaw",
                    std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
                ))
                .format(config::FileFormat::Yaml)
                .required(false),
            )
            // Add environment variables with JACKDAW_ prefix
            .add_source(
                config::Environment::with_prefix("JACKDAW")
                    .separator("__")
                    .try_parsing(true),
            );

        let config = config_builder.build()?;
        config.try_deserialize()
    }
}
