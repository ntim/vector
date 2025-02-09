use bytes::{BufMut, BytesMut};
use prost::Message;
use snafu::Snafu;
use tokio_util::codec::Encoder;
use vector_config::configurable_component;
use vector_core::event::{proto, Event};

use crate::{
    config::{AcknowledgementsConfig, GenerateConfig},
    sinks::{util::tcp::TcpSinkConfig, Healthcheck, VectorSink},
    tcp::TcpKeepaliveConfig,
    tls::TlsEnableableConfig,
};

/// Configuration for version one of the `vector` sink.
#[configurable_component]
#[derive(Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct VectorConfig {
    /// The downstream Vector address to connect to.
    ///
    /// The address _must_ include a port.
    address: String,

    #[configurable(derived)]
    keepalive: Option<TcpKeepaliveConfig>,

    #[configurable(derived)]
    tls: Option<TlsEnableableConfig>,

    /// The size, in bytes, of the socket's send buffer.
    ///
    /// If set, the value of the setting is passed via the `SO_SNDBUF` option.
    send_buffer_bytes: Option<usize>,

    #[configurable(derived)]
    #[serde(
        default,
        deserialize_with = "crate::serde::bool_or_struct",
        skip_serializing_if = "crate::serde::skip_serializing_if_default"
    )]
    pub(super) acknowledgements: AcknowledgementsConfig,
}

impl VectorConfig {
    pub fn set_tls(&mut self, config: Option<TlsEnableableConfig>) {
        self.tls = config;
    }

    pub const fn new(
        address: String,
        keepalive: Option<TcpKeepaliveConfig>,
        tls: Option<TlsEnableableConfig>,
        send_buffer_bytes: Option<usize>,
        acknowledgements: AcknowledgementsConfig,
    ) -> Self {
        Self {
            address,
            keepalive,
            tls,
            send_buffer_bytes,
            acknowledgements,
        }
    }

    pub const fn from_address(address: String, acknowledgements: AcknowledgementsConfig) -> Self {
        Self::new(address, None, None, None, acknowledgements)
    }
}

#[derive(Debug, Snafu)]
enum BuildError {
    #[snafu(display("Missing host in address field"))]
    MissingHost,
    #[snafu(display("Missing port in address field"))]
    MissingPort,
}

impl GenerateConfig for VectorConfig {
    fn generate_config() -> toml::Value {
        toml::Value::try_from(Self::new(
            "127.0.0.1:5000".to_string(),
            None,
            None,
            None,
            Default::default(),
        ))
        .unwrap()
    }
}

impl VectorConfig {
    pub(crate) async fn build(&self) -> crate::Result<(VectorSink, Healthcheck)> {
        let sink_config = TcpSinkConfig::new(
            self.address.clone(),
            self.keepalive,
            self.tls.clone(),
            self.send_buffer_bytes,
        );
        sink_config.build(Default::default(), VectorEncoder)
    }
}

#[derive(Debug, Clone)]
struct VectorEncoder;

impl Encoder<Event> for VectorEncoder {
    type Error = codecs::encoding::Error;

    fn encode(&mut self, event: Event, out: &mut BytesMut) -> Result<(), Self::Error> {
        let data = proto::EventWrapper::from(event);
        let event_len = data.encoded_len();
        let full_len = event_len + 4;

        let capacity = out.capacity();
        if capacity < full_len {
            out.reserve(full_len - capacity);
        }
        out.put_u32(event_len as u32);
        data.encode(out).unwrap();

        Ok(())
    }
}

#[derive(Debug, Snafu)]
enum HealthcheckError {
    #[snafu(display("Connect error: {}", source))]
    ConnectError { source: std::io::Error },
}

#[cfg(test)]
mod test {
    use futures::{future::ready, stream};
    use vector_core::event::{Event, LogEvent};

    use crate::{
        config::GenerateConfig,
        test_util::{
            components::{run_and_assert_sink_compliance, SINK_TAGS},
            next_addr, wait_for_tcp, CountReceiver,
        },
        tls::TlsEnableableConfig,
    };

    use super::VectorConfig;

    #[test]
    fn generate_config() {
        crate::test_util::test_generate_config::<super::VectorConfig>();
    }

    #[tokio::test]
    async fn component_spec_compliance() {
        let mock_endpoint_addr = next_addr();
        let _receiver = CountReceiver::receive_lines(mock_endpoint_addr);

        wait_for_tcp(mock_endpoint_addr).await;

        let config = VectorConfig::generate_config().to_string();
        let mut config = toml::from_str::<VectorConfig>(&config).expect("config should be valid");
        config.address = mock_endpoint_addr.to_string();
        config.tls = Some(TlsEnableableConfig::default());

        let (sink, _healthcheck) = config.build().await.unwrap();

        let event = Event::Log(LogEvent::from("simple message"));
        run_and_assert_sink_compliance(sink, stream::once(ready(event)), &SINK_TAGS).await;
    }
}
