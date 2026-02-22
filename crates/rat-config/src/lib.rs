mod compat;
mod generated;
mod model;
mod paths;
mod store;
mod validate;

pub use generated::{load_generated_or_default, save_generated};
pub use model::{
    ArtifactsConfig, ConfigError, FieldDef, FoxgloveOutputConfig, GeneratedConfig, GeneratedMeta,
    GeneratedPacketDef, GenerationConfig, JsonlOutputConfig, PacketDef, ProjectConfig,
    RatitudeConfig, RttdBehaviorConfig, RttdConfig, RttdOutputsConfig, RttdSourceConfig,
    DEFAULT_CONFIG_PATH, DEFAULT_GENERATED_HEADER_NAME, DEFAULT_GENERATED_TOML_NAME,
};
pub use paths::{resolve_config_paths, ConfigPaths};
pub use store::{load, load_or_default, ConfigStore};

#[cfg(test)]
mod tests;
