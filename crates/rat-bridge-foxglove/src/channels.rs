use std::sync::Arc;

use anyhow::{anyhow, Result};
use foxglove::schemas::RawImage;
use foxglove::{Channel, Context, RawChannel, Schema};

use crate::binding::{PacketBinding, DEFAULT_MARKER_SCHEMA, DEFAULT_TRANSFORM_SCHEMA};

#[derive(Clone)]
pub(crate) struct PacketChannels {
    pub(crate) data: Arc<RawChannel>,
    pub(crate) marker: Option<Arc<RawChannel>>,
    pub(crate) tf: Option<Arc<RawChannel>>,
    pub(crate) image: Option<Arc<Channel<RawImage>>>,
    pub(crate) binding: PacketBinding,
}

pub(crate) fn build_packet_channels(
    context: &Arc<Context>,
    bindings: &[PacketBinding],
) -> Result<Vec<PacketChannels>> {
    let mut out = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let data = build_raw_channel(
            context,
            &binding.topic,
            &binding.schema_name,
            &binding.schema_json,
        )?;

        let marker = match &binding.marker_topic {
            Some(topic) => Some(build_raw_channel(
                context,
                topic,
                "visualization_msgs/Marker",
                DEFAULT_MARKER_SCHEMA,
            )?),
            None => None,
        };

        let tf = match &binding.tf_topic {
            Some(topic) => Some(build_raw_channel(
                context,
                topic,
                "foxglove.FrameTransforms",
                DEFAULT_TRANSFORM_SCHEMA,
            )?),
            None => None,
        };

        let image = binding
            .image_topic
            .as_ref()
            .map(|topic| Arc::new(context.channel_builder(topic).build::<RawImage>()));

        out.push(PacketChannels {
            data,
            marker,
            tf,
            image,
            binding: binding.clone(),
        });
    }
    Ok(out)
}

fn build_raw_channel(
    context: &Arc<Context>,
    topic: &str,
    schema_name: &str,
    schema_json: &str,
) -> Result<Arc<RawChannel>> {
    context
        .channel_builder(topic)
        .message_encoding("json")
        .schema(Some(Schema::new(
            schema_name,
            "jsonschema",
            schema_json.as_bytes().to_vec(),
        )))
        .build_raw()
        .map_err(|err| anyhow!(err.to_string()))
}
