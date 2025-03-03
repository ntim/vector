use codecs::decoding::{DeserializerConfig, FramingConfig};
use vector_config::configurable_component;
use vector_core::config::LogNamespace;

use crate::aws::create_client;
use crate::codecs::DecodingConfig;
use crate::common::sqs::SqsClientBuilder;
use crate::tls::TlsConfig;
use crate::{
    aws::{auth::AwsAuthentication, region::RegionOrEndpoint},
    config::{AcknowledgementsConfig, Output, SourceConfig, SourceContext},
    serde::{bool_or_struct, default_decoding, default_framing_message_based},
    sources::aws_sqs::source::SqsSource,
};

/// Configuration for the `aws_sqs` source.
#[configurable_component(source)]
#[derive(Clone, Debug, Derivative)]
#[derivative(Default)]
#[serde(deny_unknown_fields)]
pub struct AwsSqsConfig {
    #[serde(flatten)]
    pub region: RegionOrEndpoint,

    #[configurable(derived)]
    #[serde(default)]
    pub auth: AwsAuthentication,

    /// The URL of the SQS queue to poll for messages.
    pub queue_url: String,

    /// How long to wait while polling the queue for new messages, in seconds.
    ///
    /// Generally should not be changed unless instructed to do so, as if messages are available, they will always be
    /// consumed, regardless of the value of `poll_secs`.
    // NOTE: We restrict this to u32 for safe conversion to i64 later.
    #[serde(default = "default_poll_secs")]
    #[derivative(Default(value = "default_poll_secs()"))]
    pub poll_secs: u32,

    /// The visibility timeout to use for messages, in secords.
    ///
    /// This controls how long a message is left unavailable after Vector receives it. If Vector receives a message, and
    /// takes longer than `visibility_timeout_secs` to process and delete the message from the queue, it will be made reavailable for another consumer.
    ///
    /// This can happen if, for example, if Vector crashes between consuming a message and deleting it.
    // NOTE: We restrict this to u32 for safe conversion to i64 later.
    // restricted to u32 for safe conversion to i64 later
    #[serde(default = "default_visibility_timeout_secs")]
    #[derivative(Default(value = "default_visibility_timeout_secs()"))]
    pub(super) visibility_timeout_secs: u32,

    /// Whether to delete the message once Vector processes it.
    ///
    /// It can be useful to set this to `false` to debug or during initial Vector setup.
    #[serde(default = "default_true")]
    #[derivative(Default(value = "default_true()"))]
    pub(super) delete_message: bool,

    /// Number of concurrent tasks to create for polling the queue for messages.
    ///
    /// Defaults to the number of available CPUs on the system.
    ///
    /// Should not typically need to be changed, but it can sometimes be beneficial to raise this value when there is a
    /// high rate of messages being pushed into the queue and the messages being fetched are small. In these cases,
    /// Vector may not fully utilize system resources without fetching more messages per second, as it spends more time
    /// fetching the messages than processing them.
    #[serde(default = "default_client_concurrency")]
    #[derivative(Default(value = "default_client_concurrency()"))]
    pub client_concurrency: u32,

    #[configurable(derived)]
    #[serde(default = "default_framing_message_based")]
    #[derivative(Default(value = "default_framing_message_based()"))]
    pub framing: FramingConfig,

    #[configurable(derived)]
    #[serde(default = "default_decoding")]
    #[derivative(Default(value = "default_decoding()"))]
    pub decoding: DeserializerConfig,

    #[configurable(derived)]
    #[serde(default, deserialize_with = "bool_or_struct")]
    pub acknowledgements: AcknowledgementsConfig,

    #[configurable(derived)]
    pub tls: Option<TlsConfig>,
}

#[async_trait::async_trait]
#[typetag::serde(name = "aws_sqs")]
impl SourceConfig for AwsSqsConfig {
    async fn build(&self, cx: SourceContext) -> crate::Result<crate::sources::Source> {
        let client = self.build_client(&cx).await?;
        let decoder = DecodingConfig::new(
            self.framing.clone(),
            self.decoding.clone(),
            LogNamespace::Legacy,
        )
        .build();
        let acknowledgements = cx.do_acknowledgements(&self.acknowledgements);

        Ok(Box::pin(
            SqsSource {
                client,
                queue_url: self.queue_url.clone(),
                decoder,
                poll_secs: self.poll_secs,
                concurrency: self.client_concurrency,
                visibility_timeout_secs: self.visibility_timeout_secs,
                delete_message: self.delete_message,
                acknowledgements,
            }
            .run(cx.out, cx.shutdown),
        ))
    }

    fn outputs(&self, _global_log_namespace: LogNamespace) -> Vec<Output> {
        vec![Output::default(self.decoding.output_type())]
    }

    fn source_type(&self) -> &'static str {
        "aws_sqs"
    }

    fn can_acknowledge(&self) -> bool {
        true
    }
}

impl AwsSqsConfig {
    async fn build_client(&self, cx: &SourceContext) -> crate::Result<aws_sdk_sqs::Client> {
        create_client::<SqsClientBuilder>(
            &self.auth,
            self.region.region(),
            self.region.endpoint()?,
            &cx.proxy,
            &self.tls,
            false,
        )
        .await
    }
}

const fn default_poll_secs() -> u32 {
    15
}

fn default_client_concurrency() -> u32 {
    crate::num_threads() as u32
}

const fn default_visibility_timeout_secs() -> u32 {
    300
}

const fn default_true() -> bool {
    true
}

impl_generate_config_from_default!(AwsSqsConfig);
