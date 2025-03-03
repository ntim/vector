use futures::FutureExt;
use indoc::indoc;
use tower::ServiceBuilder;
use vector_config::configurable_component;
use vector_core::config::proxy::ProxyConfig;

use crate::{
    config::{AcknowledgementsConfig, GenerateConfig, Input, SinkConfig, SinkContext},
    http::HttpClient,
    sinks::{
        datadog::{
            events::{
                service::{DatadogEventsResponse, DatadogEventsService},
                sink::DatadogEventsSink,
            },
            get_api_base_endpoint, get_api_validate_endpoint, healthcheck, Region,
        },
        util::{http::HttpStatusRetryLogic, ServiceBuilderExt, TowerRequestConfig},
        Healthcheck, VectorSink,
    },
    tls::{MaybeTlsSettings, TlsEnableableConfig},
};

/// Configuration for the `datadog_events` sink.
#[configurable_component(sink)]
#[derive(Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct DatadogEventsConfig {
    /// The endpoint to send events to.
    pub endpoint: Option<String>,

    /// The Datadog region to send events to.
    ///
    /// This option is deprecated, and the `site` field should be used instead.
    #[configurable(deprecated)]
    pub region: Option<Region>,

    /// The Datadog [site][dd_site] to send events to.
    ///
    /// [dd_site]: https://docs.datadoghq.com/getting_started/site
    pub site: Option<String>,

    /// The default Datadog [API key][api_key] to send events with.
    ///
    /// If an event has a Datadog [API key][api_key] set explicitly in its metadata, it will take
    /// precedence over the default.
    ///
    /// [api_key]: https://docs.datadoghq.com/api/?lang=bash#authentication
    pub default_api_key: String,

    #[configurable(derived)]
    pub(super) tls: Option<TlsEnableableConfig>,

    #[configurable(derived)]
    #[serde(default)]
    pub request: TowerRequestConfig,

    #[configurable(derived)]
    #[serde(
        default,
        deserialize_with = "crate::serde::bool_or_struct",
        skip_serializing_if = "crate::serde::skip_serializing_if_default"
    )]
    acknowledgements: AcknowledgementsConfig,
}

impl GenerateConfig for DatadogEventsConfig {
    fn generate_config() -> toml::Value {
        toml::from_str(indoc! {r#"
            default_api_key = "${DATADOG_API_KEY_ENV_VAR}"
        "#})
        .unwrap()
    }
}

impl DatadogEventsConfig {
    fn get_api_events_endpoint(&self) -> http::Uri {
        let api_base_endpoint =
            get_api_base_endpoint(self.endpoint.as_ref(), self.site.as_ref(), self.region);

        // We know this URI will be valid since we have just built it up ourselves.
        http::Uri::try_from(format!("{}/api/v1/events", api_base_endpoint)).expect("URI not valid")
    }

    fn build_client(&self, proxy: &ProxyConfig) -> crate::Result<HttpClient> {
        let tls = MaybeTlsSettings::from_config(&self.tls, false)?;
        let client = HttpClient::new(tls, proxy)?;
        Ok(client)
    }

    fn build_healthcheck(&self, client: HttpClient) -> crate::Result<Healthcheck> {
        let validate_endpoint =
            get_api_validate_endpoint(self.endpoint.as_ref(), self.site.as_ref(), self.region)?;
        Ok(healthcheck(client, validate_endpoint, self.default_api_key.clone()).boxed())
    }

    fn build_sink(&self, client: HttpClient) -> crate::Result<VectorSink> {
        let service = DatadogEventsService::new(
            self.get_api_events_endpoint(),
            self.default_api_key.clone(),
            client,
        );

        let request_opts = self.request;
        let request_settings = request_opts.unwrap_with(&TowerRequestConfig::default());
        let retry_logic = HttpStatusRetryLogic::new(|req: &DatadogEventsResponse| req.http_status);

        let service = ServiceBuilder::new()
            .settings(request_settings, retry_logic)
            .service(service);

        let sink = DatadogEventsSink { service };

        Ok(VectorSink::from_event_streamsink(sink))
    }
}

#[async_trait::async_trait]
#[typetag::serde(name = "datadog_events")]
impl SinkConfig for DatadogEventsConfig {
    async fn build(&self, cx: SinkContext) -> crate::Result<(VectorSink, Healthcheck)> {
        let client = self.build_client(cx.proxy())?;
        let healthcheck = self.build_healthcheck(client.clone())?;
        let sink = self.build_sink(client)?;

        Ok((sink, healthcheck))
    }

    fn input(&self) -> Input {
        Input::log()
    }

    fn sink_type(&self) -> &'static str {
        "datadog_events"
    }

    fn acknowledgements(&self) -> Option<&AcknowledgementsConfig> {
        Some(&self.acknowledgements)
    }
}

#[cfg(test)]
mod tests {
    use crate::sinks::datadog::events::config::DatadogEventsConfig;

    #[test]
    fn generate_config() {
        crate::test_util::test_generate_config::<DatadogEventsConfig>();
    }
}
