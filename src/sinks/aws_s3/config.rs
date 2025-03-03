use std::convert::TryInto;

use aws_sdk_s3::Client as S3Client;
use codecs::encoding::{Framer, FramingConfig};
use codecs::TextSerializerConfig;
use tower::ServiceBuilder;
use vector_config::configurable_component;
use vector_core::sink::VectorSink;

use crate::{
    aws::{AwsAuthentication, RegionOrEndpoint},
    codecs::{Encoder, EncodingConfigWithFraming, SinkType},
    config::{
        AcknowledgementsConfig, DataType, GenerateConfig, Input, ProxyConfig, SinkConfig,
        SinkContext,
    },
    sinks::{
        aws_s3::sink::S3RequestOptions,
        s3_common::{
            self,
            config::{S3Options, S3RetryLogic},
            service::S3Service,
            sink::S3Sink,
        },
        util::{
            partitioner::KeyPartitioner, BatchConfig, BulkSizeBasedDefaultBatchSettings,
            Compression, ServiceBuilderExt, TowerRequestConfig,
        },
        Healthcheck,
    },
    tls::TlsConfig,
};

const DEFAULT_KEY_PREFIX: &str = "date=%F/";
const DEFAULT_FILENAME_TIME_FORMAT: &str = "%s";
const DEFAULT_FILENAME_APPEND_UUID: bool = true;

/// Configuration for the `aws_s3` sink.
#[configurable_component(sink)]
#[derive(Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct S3SinkConfig {
    /// The S3 bucket name.
    ///
    /// This must not include a leading `s3://` or a trailing `/`.
    pub bucket: String,

    /// A prefix to apply to all object keys.
    ///
    /// Prefixes are useful for partitioning objects, such as by creating an object key that
    /// stores objects under a particular "directory". If using a prefix for this purpose, it must end
    /// in `/` in order to act as a directory path: Vector will **not** add a trailing `/` automatically.
    #[configurable(metadata(templateable))]
    pub key_prefix: Option<String>,

    /// The timestamp format for the time component of the object key.
    ///
    /// By default, object keys are appended with a timestamp that reflects when the objects are
    /// sent to S3, such that the resulting object key is functionally equivalent to joining the key
    /// prefix with the formatted timestamp, such as `date=2022-07-18/1658176486`.
    ///
    /// This would represent a `key_prefix` set to `date=%F/` and the timestamp of Mon Jul 18 2022
    /// 20:34:44 GMT+0000, with the `filename_time_format` being set to `%s`, which renders
    /// timestamps in seconds since the Unix epoch.
    ///
    /// Supports the common [`strftime`][chrono_strftime_specifiers] specifiers found in most
    /// languages.
    ///
    /// When set to an empty string, no timestamp will be appended to the key prefix.
    ///
    /// [chrono_strftime_specifiers]: https://docs.rs/chrono/latest/chrono/format/strftime/index.html#specifiers
    pub filename_time_format: Option<String>,

    /// Whether or not to append a UUID v4 token to the end of the object key.
    ///
    /// The UUID is appended to the timestamp portion of the object key, such that if the object key
    /// being generated was `date=2022-07-18/1658176486`, setting this field to `true` would result
    /// in an object key that looked like `date=2022-07-18/1658176486-30f6652c-71da-4f9f-800d-a1189c47c547`.
    ///
    /// This ensures there are no name collisions, and can be useful in high-volume workloads where
    /// object keys must be unique.
    pub filename_append_uuid: Option<bool>,

    /// The filename extension to use in the object key.
    pub filename_extension: Option<String>,

    #[serde(flatten)]
    pub options: S3Options,

    #[serde(flatten)]
    pub region: RegionOrEndpoint,

    #[serde(flatten)]
    pub encoding: EncodingConfigWithFraming,

    #[configurable(derived)]
    #[serde(default = "Compression::gzip_default")]
    pub compression: Compression,

    #[configurable(derived)]
    #[serde(default)]
    pub batch: BatchConfig<BulkSizeBasedDefaultBatchSettings>,

    #[configurable(derived)]
    #[serde(default)]
    pub request: TowerRequestConfig,

    #[configurable(derived)]
    pub tls: Option<TlsConfig>,

    #[configurable(derived)]
    #[serde(default)]
    pub auth: AwsAuthentication,

    #[configurable(derived)]
    #[serde(
        default,
        deserialize_with = "crate::serde::bool_or_struct",
        skip_serializing_if = "crate::serde::skip_serializing_if_default"
    )]
    pub acknowledgements: AcknowledgementsConfig,
}

impl GenerateConfig for S3SinkConfig {
    fn generate_config() -> toml::Value {
        toml::Value::try_from(Self {
            bucket: "".to_owned(),
            key_prefix: None,
            filename_time_format: None,
            filename_append_uuid: None,
            filename_extension: None,
            options: S3Options::default(),
            region: RegionOrEndpoint::default(),
            encoding: (None::<FramingConfig>, TextSerializerConfig::new()).into(),
            compression: Compression::gzip_default(),
            batch: BatchConfig::default(),
            request: TowerRequestConfig::default(),
            tls: Some(TlsConfig::default()),
            auth: AwsAuthentication::default(),
            acknowledgements: Default::default(),
        })
        .unwrap()
    }
}

#[async_trait::async_trait]
#[typetag::serde(name = "aws_s3")]
impl SinkConfig for S3SinkConfig {
    async fn build(&self, cx: SinkContext) -> crate::Result<(VectorSink, Healthcheck)> {
        let service = self.create_service(&cx.proxy).await?;
        let healthcheck = self.build_healthcheck(service.client())?;
        let sink = self.build_processor(service)?;
        Ok((sink, healthcheck))
    }

    fn input(&self) -> Input {
        Input::new(self.encoding.config().1.input_type() & DataType::Log)
    }

    fn sink_type(&self) -> &'static str {
        "aws_s3"
    }

    fn acknowledgements(&self) -> Option<&AcknowledgementsConfig> {
        Some(&self.acknowledgements)
    }
}

impl S3SinkConfig {
    pub fn build_processor(&self, service: S3Service) -> crate::Result<VectorSink> {
        // Build our S3 client/service, which is what we'll ultimately feed
        // requests into in order to ship files to S3.  We build this here in
        // order to configure the client/service with retries, concurrency
        // limits, rate limits, and whatever else the client should have.
        let request_limits = self.request.unwrap_with(&Default::default());
        let service = ServiceBuilder::new()
            .settings(request_limits, S3RetryLogic)
            .service(service);

        // Configure our partitioning/batching.
        let batch_settings = self.batch.into_batcher_settings()?;
        let key_prefix = self
            .key_prefix
            .as_ref()
            .cloned()
            .unwrap_or_else(|| DEFAULT_KEY_PREFIX.into())
            .try_into()?;
        let partitioner = KeyPartitioner::new(key_prefix);

        // And now collect all of the S3-specific options and configuration knobs.
        let filename_time_format = self
            .filename_time_format
            .as_ref()
            .cloned()
            .unwrap_or_else(|| DEFAULT_FILENAME_TIME_FORMAT.into());
        let filename_append_uuid = self
            .filename_append_uuid
            .unwrap_or(DEFAULT_FILENAME_APPEND_UUID);

        let transformer = self.encoding.transformer();
        let (framer, serializer) = self.encoding.build(SinkType::MessageBased)?;
        let encoder = Encoder::<Framer>::new(framer, serializer);

        let request_options = S3RequestOptions {
            bucket: self.bucket.clone(),
            api_options: self.options.clone(),
            filename_extension: self.filename_extension.clone(),
            filename_time_format,
            filename_append_uuid,
            encoder: (transformer, encoder),
            compression: self.compression,
        };

        let sink = S3Sink::new(service, request_options, partitioner, batch_settings);

        Ok(VectorSink::from_event_streamsink(sink))
    }

    pub fn build_healthcheck(&self, client: S3Client) -> crate::Result<Healthcheck> {
        s3_common::config::build_healthcheck(self.bucket.clone(), client)
    }

    pub async fn create_service(&self, proxy: &ProxyConfig) -> crate::Result<S3Service> {
        s3_common::config::create_service(&self.region, &self.auth, proxy, &self.tls).await
    }
}

#[cfg(test)]
mod tests {
    use super::S3SinkConfig;

    #[test]
    fn generate_config() {
        crate::test_util::test_generate_config::<S3SinkConfig>();
    }
}
