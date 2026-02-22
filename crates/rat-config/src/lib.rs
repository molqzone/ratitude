mod compat;
mod model;
mod paths;
mod store;
mod validate;

pub use model::{
    ArtifactsConfig, ConfigError, FieldDef, FoxgloveOutputConfig, GeneratedConfig, GeneratedMeta,
    GeneratedPacketDef, GenerationConfig, JsonlOutputConfig, PacketDef, ProjectConfig,
    RatdBehaviorConfig, RatdConfig, RatdOutputsConfig, RatdSourceConfig, RatitudeConfig,
    DEFAULT_CONFIG_PATH, DEFAULT_GENERATED_HEADER_NAME,
};
pub use paths::{resolve_config_paths, ConfigPaths};
pub use store::{load, load_or_default, ConfigStore};

#[cfg(test)]
mod tests;
