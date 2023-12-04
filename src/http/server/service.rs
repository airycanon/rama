use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use hyper::server::conn::http2::Builder as H2ConnBuilder;
use hyper::{rt::Timer, server::conn::http1::Builder as Http1ConnBuilder};
use hyper_util::server::conn::auto::Builder as AutoConnBuilder;

use hyper_util::server::conn::auto::Http1Builder as InnerAutoHttp1Builder;
use hyper_util::server::conn::auto::Http2Builder as InnerAutoHttp2Builder;

use crate::service::{http::ServiceBuilderExt, util::Identity, Layer, Service, ServiceBuilder};
use crate::{tcp::TcpStream, BoxError};

use super::hyper_conn::HyperConnServer;
use super::{GlobalExecutor, HyperBody, Request, Response, ServeResult};

#[derive(Debug)]
pub struct HttpServer<B, L> {
    builder: B,
    service_builder: ServiceBuilder<L>,
}

impl<B, L> Clone for HttpServer<B, L>
where
    B: Clone,
    L: Clone,
{
    fn clone(&self) -> Self {
        Self {
            builder: self.builder.clone(),
            service_builder: self.service_builder.clone(),
        }
    }
}

impl HttpServer<Http1ConnBuilder, Identity> {
    /// Create a new http/1.1 `Builder` with default settings.
    pub fn http1() -> Self {
        Self {
            builder: Http1ConnBuilder::new(),
            service_builder: ServiceBuilder::new(),
        }
    }
}

impl<L> HttpServer<Http1ConnBuilder, L> {
    /// Http1 configuration.
    pub fn http1_mut(&mut self) -> Http1Config<'_> {
        Http1Config {
            inner: &mut self.builder,
        }
    }
}

/// A configuration builder for HTTP/1 server connections.
pub struct Http1Config<'a> {
    inner: &'a mut Http1ConnBuilder,
}

impl<'a> Http1Config<'a> {
    /// Set whether HTTP/1 connections should support half-closures.
    ///
    /// Clients can chose to shutdown their write-side while waiting
    /// for the server to respond. Setting this to `true` will
    /// prevent closing the connection immediately if `read`
    /// detects an EOF in the middle of a request.
    ///
    /// Default is `false`.
    pub fn half_close(&mut self, val: bool) -> &mut Self {
        self.inner.half_close(val);
        self
    }

    /// Enables or disables HTTP/1 keep-alive.
    ///
    /// Default is true.
    pub fn keep_alive(&mut self, val: bool) -> &mut Self {
        self.inner.keep_alive(val);
        self
    }

    /// Set whether HTTP/1 connections will write header names as title case at
    /// the socket level.
    ///
    /// Note that this setting does not affect HTTP/2.
    ///
    /// Default is false.
    pub fn title_case_headers(&mut self, enabled: bool) -> &mut Self {
        self.inner.title_case_headers(enabled);
        self
    }

    /// Set whether to support preserving original header cases.
    ///
    /// Currently, this will record the original cases received, and store them
    /// in a private extension on the `Request`. It will also look for and use
    /// such an extension in any provided `Response`.
    ///
    /// Since the relevant extension is still private, there is no way to
    /// interact with the original cases. The only effect this can have now is
    /// to forward the cases in a proxy-like fashion.
    ///
    /// Note that this setting does not affect HTTP/2.
    ///
    /// Default is false.
    pub fn preserve_header_case(&mut self, enabled: bool) -> &mut Self {
        self.inner.preserve_header_case(enabled);
        self
    }

    /// Set a timeout for reading client request headers. If a client does not
    /// transmit the entire header within this time, the connection is closed.
    ///
    /// Default is None.
    pub fn header_read_timeout(&mut self, read_timeout: Duration) -> &mut Self {
        self.inner.header_read_timeout(read_timeout);
        self
    }

    /// Set whether HTTP/1 connections should try to use vectored writes,
    /// or always flatten into a single buffer.
    ///
    /// Note that setting this to false may mean more copies of body data,
    /// but may also improve performance when an IO transport doesn't
    /// support vectored writes well, such as most TLS implementations.
    ///
    /// Setting this to true will force hyper to use queued strategy
    /// which may eliminate unnecessary cloning on some TLS backends
    ///
    /// Default is `auto`. In this mode hyper will try to guess which
    /// mode to use
    pub fn writev(&mut self, val: bool) -> &mut Self {
        self.inner.writev(val);
        self
    }

    /// Set the maximum buffer size for the connection.
    ///
    /// Default is ~400kb.
    ///
    /// # Panics
    ///
    /// The minimum value allowed is 8192. This method panics if the passed `max` is less than the minimum.
    pub fn max_buf_size(&mut self, max: usize) -> &mut Self {
        self.inner.max_buf_size(max);
        self
    }

    /// Aggregates flushes to better support pipelined responses.
    ///
    /// Experimental, may have bugs.
    ///
    /// Default is false.
    pub fn pipeline_flush(&mut self, enabled: bool) -> &mut Self {
        self.inner.pipeline_flush(enabled);
        self
    }

    /// Set the timer used in background tasks.
    pub fn timer<M>(&mut self, timer: M) -> &mut Self
    where
        M: Timer + Send + Sync + 'static,
    {
        self.inner.timer(timer);
        self
    }
}

impl HttpServer<H2ConnBuilder<GlobalExecutor>, Identity> {
    /// Create a new h2 `Builder` with default settings.
    pub fn h2() -> Self {
        Self {
            builder: H2ConnBuilder::new(GlobalExecutor::new()),
            service_builder: ServiceBuilder::new(),
        }
    }
}

impl<E, L> HttpServer<H2ConnBuilder<E>, L> {
    /// H2 configuration.
    pub fn h2_mut(&mut self) -> H2Config<'_, E> {
        H2Config {
            inner: &mut self.builder,
        }
    }
}

/// A configuration builder for HTTP/2 server connections.
pub struct H2Config<'a, E> {
    inner: &'a mut H2ConnBuilder<E>,
}

impl<'a, E> H2Config<'a, E> {
    /// Sets the [`SETTINGS_INITIAL_WINDOW_SIZE`][spec] option for HTTP2
    /// stream-level flow control.
    ///
    /// Passing `None` will do nothing.
    ///
    /// If not set, hyper will use a default.
    ///
    /// [spec]: https://http2.github.io/http2-spec/#SETTINGS_INITIAL_WINDOW_SIZE
    pub fn initial_stream_window_size(&mut self, sz: impl Into<Option<u32>>) -> &mut Self {
        self.inner.initial_stream_window_size(sz);
        self
    }

    /// Sets the max connection-level flow control for HTTP2.
    ///
    /// Passing `None` will do nothing.
    ///
    /// If not set, hyper will use a default.
    pub fn initial_connection_window_size(&mut self, sz: impl Into<Option<u32>>) -> &mut Self {
        self.inner.initial_connection_window_size(sz);
        self
    }

    /// Sets whether to use an adaptive flow control.
    ///
    /// Enabling this will override the limits set in
    /// `http2_initial_stream_window_size` and
    /// `http2_initial_connection_window_size`.
    pub fn adaptive_window(&mut self, enabled: bool) -> &mut Self {
        self.inner.adaptive_window(enabled);
        self
    }

    /// Sets the maximum frame size to use for HTTP2.
    ///
    /// Passing `None` will do nothing.
    ///
    /// If not set, hyper will use a default.
    pub fn max_frame_size(&mut self, sz: impl Into<Option<u32>>) -> &mut Self {
        self.inner.max_frame_size(sz);
        self
    }

    /// Sets the [`SETTINGS_MAX_CONCURRENT_STREAMS`][spec] option for HTTP2
    /// connections.
    ///
    /// Default is no limit (`std::u32::MAX`). Passing `None` will do nothing.
    ///
    /// [spec]: https://http2.github.io/http2-spec/#SETTINGS_MAX_CONCURRENT_STREAMS
    pub fn max_concurrent_streams(&mut self, max: impl Into<Option<u32>>) -> &mut Self {
        self.inner.max_concurrent_streams(max);
        self
    }

    /// Sets an interval for HTTP2 Ping frames should be sent to keep a
    /// connection alive.
    ///
    /// Pass `None` to disable HTTP2 keep-alive.
    ///
    /// Default is currently disabled.
    ///
    /// # Cargo Feature
    ///
    pub fn keep_alive_interval(&mut self, interval: impl Into<Option<Duration>>) -> &mut Self {
        self.inner.keep_alive_interval(interval);
        self
    }

    /// Sets a timeout for receiving an acknowledgement of the keep-alive ping.
    ///
    /// If the ping is not acknowledged within the timeout, the connection will
    /// be closed. Does nothing if `http2_keep_alive_interval` is disabled.
    ///
    /// Default is 20 seconds.
    ///
    /// # Cargo Feature
    ///
    pub fn keep_alive_timeout(&mut self, timeout: Duration) -> &mut Self {
        self.inner.keep_alive_timeout(timeout);
        self
    }

    /// Set the maximum write buffer size for each HTTP/2 stream.
    ///
    /// Default is currently ~400KB, but may change.
    ///
    /// # Panics
    ///
    /// The value must be no larger than `u32::MAX`.
    pub fn max_send_buf_size(&mut self, max: usize) -> &mut Self {
        self.inner.max_send_buf_size(max);
        self
    }

    /// Enables the [extended CONNECT protocol].
    ///
    /// [extended CONNECT protocol]: https://datatracker.ietf.org/doc/html/rfc8441#section-4
    pub fn enable_connect_protocol(&mut self) -> &mut Self {
        self.inner.enable_connect_protocol();
        self
    }

    /// Sets the max size of received header frames.
    ///
    /// Default is currently ~16MB, but may change.
    pub fn max_header_list_size(&mut self, max: u32) -> &mut Self {
        self.inner.max_header_list_size(max);
        self
    }

    /// Set the timer used in background tasks.
    pub fn timer<M>(&mut self, timer: M) -> &mut Self
    where
        M: Timer + Send + Sync + 'static,
    {
        self.inner.timer(timer);
        self
    }
}

impl HttpServer<AutoConnBuilder<GlobalExecutor>, Identity> {
    /// Create a new dual http/1.1 + h2 `Builder` with default settings.
    pub fn auto() -> Self {
        Self {
            builder: AutoConnBuilder::new(GlobalExecutor::new()),
            service_builder: ServiceBuilder::new(),
        }
    }
}

impl<E, L> HttpServer<AutoConnBuilder<E>, L> {
    /// Http1 configuration.
    pub fn http1_mut(&mut self) -> AutoHttp1Config<'_, E> {
        AutoHttp1Config {
            inner: self.builder.http1(),
        }
    }

    /// H2 configuration.
    pub fn h2_mut(&mut self) -> AutoH2Config<'_, E> {
        AutoH2Config {
            inner: self.builder.http2(),
        }
    }
}

pub struct AutoHttp1Config<'a, E> {
    inner: InnerAutoHttp1Builder<'a, E>,
}

impl<'a, E> AutoHttp1Config<'a, E> {
    /// Set whether HTTP/1 connections should support half-closures.
    ///
    /// Clients can chose to shutdown their write-side while waiting
    /// for the server to respond. Setting this to `true` will
    /// prevent closing the connection immediately if `read`
    /// detects an EOF in the middle of a request.
    ///
    /// Default is `false`.
    pub fn half_close(&mut self, val: bool) -> &mut Self {
        self.inner.half_close(val);
        self
    }

    /// Enables or disables HTTP/1 keep-alive.
    ///
    /// Default is true.
    pub fn keep_alive(&mut self, val: bool) -> &mut Self {
        self.inner.keep_alive(val);
        self
    }

    /// Set whether HTTP/1 connections will write header names as title case at
    /// the socket level.
    ///
    /// Note that this setting does not affect HTTP/2.
    ///
    /// Default is false.
    pub fn title_case_headers(&mut self, enabled: bool) -> &mut Self {
        self.inner.title_case_headers(enabled);
        self
    }

    /// Set whether to support preserving original header cases.
    ///
    /// Currently, this will record the original cases received, and store them
    /// in a private extension on the `Request`. It will also look for and use
    /// such an extension in any provided `Response`.
    ///
    /// Since the relevant extension is still private, there is no way to
    /// interact with the original cases. The only effect this can have now is
    /// to forward the cases in a proxy-like fashion.
    ///
    /// Note that this setting does not affect HTTP/2.
    ///
    /// Default is false.
    pub fn preserve_header_case(&mut self, enabled: bool) -> &mut Self {
        self.inner.preserve_header_case(enabled);
        self
    }

    /// Set a timeout for reading client request headers. If a client does not
    /// transmit the entire header within this time, the connection is closed.
    ///
    /// Default is None.
    pub fn header_read_timeout(&mut self, read_timeout: Duration) -> &mut Self {
        self.inner.header_read_timeout(read_timeout);
        self
    }

    /// Set whether HTTP/1 connections should try to use vectored writes,
    /// or always flatten into a single buffer.
    ///
    /// Note that setting this to false may mean more copies of body data,
    /// but may also improve performance when an IO transport doesn't
    /// support vectored writes well, such as most TLS implementations.
    ///
    /// Setting this to true will force hyper to use queued strategy
    /// which may eliminate unnecessary cloning on some TLS backends
    ///
    /// Default is `auto`. In this mode hyper will try to guess which
    /// mode to use
    pub fn writev(&mut self, val: bool) -> &mut Self {
        self.inner.writev(val);
        self
    }

    /// Set the maximum buffer size for the connection.
    ///
    /// Default is ~400kb.
    ///
    /// # Panics
    ///
    /// The minimum value allowed is 8192. This method panics if the passed `max` is less than the minimum.
    pub fn max_buf_size(&mut self, max: usize) -> &mut Self {
        self.inner.max_buf_size(max);
        self
    }

    /// Aggregates flushes to better support pipelined responses.
    ///
    /// Experimental, may have bugs.
    ///
    /// Default is false.
    pub fn pipeline_flush(&mut self, enabled: bool) -> &mut Self {
        self.inner.pipeline_flush(enabled);
        self
    }

    /// Set the timer used in background tasks.
    pub fn timer<M>(&mut self, timer: M) -> &mut Self
    where
        M: Timer + Send + Sync + 'static,
    {
        self.inner.timer(timer);
        self
    }
}

pub struct AutoH2Config<'a, E> {
    inner: InnerAutoHttp2Builder<'a, E>,
}

impl<'a, E> AutoH2Config<'a, E> {
    /// Sets the [`SETTINGS_INITIAL_WINDOW_SIZE`][spec] option for HTTP2
    /// stream-level flow control.
    ///
    /// Passing `None` will do nothing.
    ///
    /// If not set, hyper will use a default.
    ///
    /// [spec]: https://http2.github.io/http2-spec/#SETTINGS_INITIAL_WINDOW_SIZE
    pub fn initial_stream_window_size(&mut self, sz: impl Into<Option<u32>>) -> &mut Self {
        self.inner.initial_stream_window_size(sz);
        self
    }

    /// Sets the max connection-level flow control for HTTP2.
    ///
    /// Passing `None` will do nothing.
    ///
    /// If not set, hyper will use a default.
    pub fn initial_connection_window_size(&mut self, sz: impl Into<Option<u32>>) -> &mut Self {
        self.inner.initial_connection_window_size(sz);
        self
    }

    /// Sets whether to use an adaptive flow control.
    ///
    /// Enabling this will override the limits set in
    /// `http2_initial_stream_window_size` and
    /// `http2_initial_connection_window_size`.
    pub fn adaptive_window(&mut self, enabled: bool) -> &mut Self {
        self.inner.adaptive_window(enabled);
        self
    }

    /// Sets the maximum frame size to use for HTTP2.
    ///
    /// Passing `None` will do nothing.
    ///
    /// If not set, hyper will use a default.
    pub fn max_frame_size(&mut self, sz: impl Into<Option<u32>>) -> &mut Self {
        self.inner.max_frame_size(sz);
        self
    }

    /// Sets the [`SETTINGS_MAX_CONCURRENT_STREAMS`][spec] option for HTTP2
    /// connections.
    ///
    /// Default is no limit (`std::u32::MAX`). Passing `None` will do nothing.
    ///
    /// [spec]: https://http2.github.io/http2-spec/#SETTINGS_MAX_CONCURRENT_STREAMS
    pub fn max_concurrent_streams(&mut self, max: impl Into<Option<u32>>) -> &mut Self {
        self.inner.max_concurrent_streams(max);
        self
    }

    /// Sets an interval for HTTP2 Ping frames should be sent to keep a
    /// connection alive.
    ///
    /// Pass `None` to disable HTTP2 keep-alive.
    ///
    /// Default is currently disabled.
    ///
    /// # Cargo Feature
    ///
    pub fn keep_alive_interval(&mut self, interval: impl Into<Option<Duration>>) -> &mut Self {
        self.inner.keep_alive_interval(interval);
        self
    }

    /// Sets a timeout for receiving an acknowledgement of the keep-alive ping.
    ///
    /// If the ping is not acknowledged within the timeout, the connection will
    /// be closed. Does nothing if `http2_keep_alive_interval` is disabled.
    ///
    /// Default is 20 seconds.
    ///
    /// # Cargo Feature
    ///
    pub fn keep_alive_timeout(&mut self, timeout: Duration) -> &mut Self {
        self.inner.keep_alive_timeout(timeout);
        self
    }

    /// Set the maximum write buffer size for each HTTP/2 stream.
    ///
    /// Default is currently ~400KB, but may change.
    ///
    /// # Panics
    ///
    /// The value must be no larger than `u32::MAX`.
    pub fn max_send_buf_size(&mut self, max: usize) -> &mut Self {
        self.inner.max_send_buf_size(max);
        self
    }

    /// Enables the [extended CONNECT protocol].
    ///
    /// [extended CONNECT protocol]: https://datatracker.ietf.org/doc/html/rfc8441#section-4
    pub fn enable_connect_protocol(&mut self) -> &mut Self {
        self.inner.enable_connect_protocol();
        self
    }

    /// Sets the max size of received header frames.
    ///
    /// Default is currently ~16MB, but may change.
    pub fn max_header_list_size(&mut self, max: u32) -> &mut Self {
        self.inner.max_header_list_size(max);
        self
    }

    /// Set the timer used in background tasks.
    pub fn timer<M>(&mut self, timer: M) -> &mut Self
    where
        M: Timer + Send + Sync + 'static,
    {
        self.inner.timer(timer);
        self
    }
}

impl<B, L> HttpServer<B, L> {
    /// Add a layer to the connector's service stack.
    pub fn layer<T>(self, layer: T) -> HttpServer<B, crate::service::util::Stack<T, L>>
    where
        T: Layer<L>,
    {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.layer(layer),
        }
    }
}

impl<B, L> HttpServer<B, L> {
    /// Fail requests that take longer than `timeout`.
    pub fn timeout(
        self,
        timeout: std::time::Duration,
    ) -> HttpServer<B, crate::service::util::Stack<crate::service::timeout::TimeoutLayer, L>> {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.timeout(timeout),
        }
    }
    // Conditionally reject requests based on `predicate`.
    ///
    /// `predicate` must implement the [`Predicate`] trait.
    ///
    /// This wraps the inner service with an instance of the [`Filter`]
    /// middleware.
    ///
    /// [`Filter`]: crate::service::filter::Filter
    /// [`Predicate`]: crate::service::filter::Predicate
    pub fn filter<P>(
        self,
        predicate: P,
    ) -> HttpServer<B, crate::service::util::Stack<crate::service::filter::FilterLayer<P>, L>> {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.filter(predicate),
        }
    }

    /// Conditionally reject requests based on an asynchronous `predicate`.
    ///
    /// `predicate` must implement the [`AsyncPredicate`] trait.
    ///
    /// This wraps the inner service with an instance of the [`AsyncFilter`]
    /// middleware.
    ///
    /// [`AsyncFilter`]: crate::service::filter::AsyncFilter
    /// [`AsyncPredicate`]: crate::service::filter::AsyncPredicate
    pub fn filter_async<P>(
        self,
        predicate: P,
    ) -> HttpServer<B, crate::service::util::Stack<crate::service::filter::AsyncFilterLayer<P>, L>>
    {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.filter_async(predicate),
        }
    }
}

impl<B, L> HttpServer<B, L> {
    /// Propagate a header from the request to the response.
    ///
    /// See [`tower_async_http::propagate_header`] for more details.
    ///
    /// [`tower_async_http::propagate_header`]: crate::service::http::propagate_header
    pub fn propagate_header(
        self,
        header: crate::http::HeaderName,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<
            crate::service::http::propagate_header::PropagateHeaderLayer,
            L,
        >,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.propagate_header(header),
        }
    }

    /// Add some shareable value to [request extensions].
    ///
    /// See [`tower_async_http::add_extension`] for more details.
    ///
    /// [`tower_async_http::add_extension`]: crate::service::http::add_extension
    /// [request extensions]: https://docs.rs/http/latest/http/struct.Extensions.html
    pub fn add_extension<T>(
        self,
        value: T,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::add_extension::AddExtensionLayer<T>, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.add_extension(value),
        }
    }

    /// Apply a transformation to the request body.
    ///
    /// See [`tower_async_http::map_request_body`] for more details.
    ///
    /// [`tower_async_http::map_request_body`]: crate::service::http::map_request_body
    pub fn map_request_body<F>(
        self,
        f: F,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<
            crate::service::http::map_request_body::MapRequestBodyLayer<F>,
            L,
        >,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.map_request_body(f),
        }
    }

    /// Apply a transformation to the response body.
    ///
    /// See [`tower_async_http::map_response_body`] for more details.
    ///
    /// [`tower_async_http::map_response_body`]: crate::service::http::map_response_body
    pub fn map_response_body<F>(
        self,
        f: F,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<
            crate::service::http::map_response_body::MapResponseBodyLayer<F>,
            L,
        >,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.map_response_body(f),
        }
    }

    /// Compresses response bodies.
    ///
    /// See [`tower_async_http::compression`] for more details.
    ///
    /// [`tower_async_http::compression`]: crate::service::http::compression
    pub fn compression(
        self,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::compression::CompressionLayer, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.compression(),
        }
    }

    /// Decompress response bodies.
    ///
    /// See [`tower_async_http::decompression`] for more details.
    ///
    /// [`tower_async_http::decompression`]: crate::service::http::decompression
    pub fn decompression(
        self,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::decompression::DecompressionLayer, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.decompression(),
        }
    }

    /// High level tracing that classifies responses using HTTP status codes.
    ///
    /// This method does not support customizing the output, to do that use [`TraceLayer`]
    /// instead.
    ///
    /// See [`tower_http::trace`] for more details.
    ///
    /// [`tower_http::trace`]: crate::service::http::trace
    /// [`TraceLayer`]: crate::service::http::trace::TraceLayer
    pub fn trace(
        self,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<
            crate::service::http::trace::TraceLayer<
                crate::service::http::classify::SharedClassifier<
                    crate::service::http::classify::ServerErrorsAsFailures,
                >,
            >,
            L,
        >,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.trace_for_http(),
        }
    }

    /// High level tracing that classifies responses using HTTP status codes.
    ///
    /// This method does not support customizing the output, to do that use [`TraceLayer`]
    /// instead.
    ///
    /// See [`tower_http::trace`] for more details.
    ///
    /// [`tower_http::trace`]: crate::service::http::trace
    /// [`TraceLayer`]: crate::service::http::trace::TraceLayer
    #[allow(clippy::type_complexity)]
    pub fn trace_layer<M, MakeSpan, OnRequest, OnResponse, OnBodyChunk, OnEos, OnFailure>(
        self,
        layer: crate::service::http::trace::TraceLayer<
            M,
            MakeSpan,
            OnRequest,
            OnResponse,
            OnBodyChunk,
            OnEos,
            OnFailure,
        >,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<
            crate::service::http::trace::TraceLayer<
                M,
                MakeSpan,
                OnRequest,
                OnResponse,
                OnBodyChunk,
                OnEos,
                OnFailure,
            >,
            L,
        >,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.layer(layer),
        }
    }

    /// Follow redirect responses using the [`Standard`] policy.
    ///
    /// See [`tower_async_http::follow_redirect`] for more details.
    ///
    /// [`tower_async_http::follow_redirect`]: crate::service::http::follow_redirect
    /// [`Standard`]: crate::service::http::follow_redirect::policy::Standard
    pub fn follow_redirects(
        self,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<
            crate::service::http::follow_redirect::FollowRedirectLayer<
                crate::service::http::follow_redirect::policy::Standard,
            >,
            L,
        >,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.follow_redirects(),
        }
    }

    /// Mark headers as [sensitive] on both requests and responses.
    ///
    /// See [`tower_async_http::sensitive_headers`] for more details.
    ///
    /// [sensitive]: https://docs.rs/http/latest/http/header/struct.HeaderValue.html#method.set_sensitive
    /// [`tower_async_http::sensitive_headers`]: crate::service::http::sensitive_headers
    pub fn sensitive_headers<I>(
        self,
        headers: I,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<
            crate::service::http::sensitive_headers::SetSensitiveHeadersLayer,
            L,
        >,
    >
    where
        I: IntoIterator<Item = crate::http::HeaderName>,
    {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.sensitive_headers(headers),
        }
    }

    /// Mark headers as [sensitive] on both requests.
    ///
    /// See [`tower_async_http::sensitive_headers`] for more details.
    ///
    /// [sensitive]: https://docs.rs/http/latest/http/header/struct.HeaderValue.html#method.set_sensitive
    /// [`tower_async_http::sensitive_headers`]: crate::service::http::sensitive_headers
    pub fn sensitive_request_headers(
        self,
        headers: std::sync::Arc<[crate::http::HeaderName]>,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<
            crate::service::http::sensitive_headers::SetSensitiveRequestHeadersLayer,
            L,
        >,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.sensitive_request_headers(headers),
        }
    }

    /// Mark headers as [sensitive] on both responses.
    ///
    /// See [`tower_async_http::sensitive_headers`] for more details.
    ///
    /// [sensitive]: https://docs.rs/http/latest/http/header/struct.HeaderValue.html#method.set_sensitive
    /// [`tower_async_http::sensitive_headers`]: crate::service::http::sensitive_headers
    pub fn sensitive_response_headers(
        self,
        headers: std::sync::Arc<[crate::http::HeaderName]>,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<
            crate::service::http::sensitive_headers::SetSensitiveResponseHeadersLayer,
            L,
        >,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.sensitive_response_headers(headers),
        }
    }

    /// Insert a header into the request.
    ///
    /// If a previous value exists for the same header, it is removed and replaced with the new
    /// header value.
    ///
    /// See [`tower_async_http::set_header`] for more details.
    ///
    /// [`tower_async_http::set_header`]: crate::service::http::set_header
    pub fn override_request_header<M>(
        self,
        header_name: crate::http::HeaderName,
        make: M,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::set_header::SetRequestHeaderLayer<M>, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self
                .service_builder
                .override_request_header(header_name, make),
        }
    }

    /// Append a header into the request.
    ///
    /// If previous values exist, the header will have multiple values.
    ///
    /// See [`tower_async_http::set_header`] for more details.
    ///
    /// [`tower_async_http::set_header`]: crate::service::http::set_header
    pub fn append_request_header<M>(
        self,
        header_name: crate::http::HeaderName,
        make: M,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::set_header::SetRequestHeaderLayer<M>, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self
                .service_builder
                .append_request_header(header_name, make),
        }
    }

    /// Insert a header into the request, if the header is not already present.
    ///
    /// See [`tower_async_http::set_header`] for more details.
    ///
    /// [`tower_async_http::set_header`]: crate::service::http::set_header
    pub fn insert_request_header_if_not_present<M>(
        self,
        header_name: crate::http::HeaderName,
        make: M,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::set_header::SetRequestHeaderLayer<M>, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self
                .service_builder
                .insert_request_header_if_not_present(header_name, make),
        }
    }

    /// Insert a header into the response.
    ///
    /// If a previous value exists for the same header, it is removed and replaced with the new
    /// header value.
    ///
    /// See [`tower_async_http::set_header`] for more details.
    ///
    /// [`tower_async_http::set_header`]: crate::service::http::set_header
    pub fn override_response_header<M>(
        self,
        header_name: crate::http::HeaderName,
        make: M,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::set_header::SetResponseHeaderLayer<M>, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self
                .service_builder
                .override_response_header(header_name, make),
        }
    }

    /// Append a header into the response.
    ///
    /// If previous values exist, the header will have multiple values.
    ///
    /// See [`tower_async_http::set_header`] for more details.
    ///
    /// [`tower_async_http::set_header`]: crate::service::http::set_header
    pub fn append_response_header<M>(
        self,
        header_name: crate::http::HeaderName,
        make: M,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::set_header::SetResponseHeaderLayer<M>, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self
                .service_builder
                .append_response_header(header_name, make),
        }
    }

    /// Insert a header into the response, if the header is not already present.
    ///
    /// See [`tower_async_http::set_header`] for more details.
    ///
    /// [`tower_async_http::set_header`]: crate::service::http::set_header
    pub fn insert_response_header_if_not_present<M>(
        self,
        header_name: crate::http::HeaderName,
        make: M,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::set_header::SetResponseHeaderLayer<M>, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self
                .service_builder
                .insert_response_header_if_not_present(header_name, make),
        }
    }

    /// Add request id header and extension.
    ///
    /// See [`tower_async_http::request_id`] for more details.
    ///
    /// [`tower_async_http::request_id`]: crate::service::http::request_id
    pub fn set_request_id<M>(
        self,
        header_name: crate::http::HeaderName,
        make_request_id: M,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::request_id::SetRequestIdLayer<M>, L>,
    >
    where
        M: crate::service::http::request_id::MakeRequestId,
    {
        HttpServer {
            builder: self.builder,
            service_builder: self
                .service_builder
                .set_request_id(header_name, make_request_id),
        }
    }

    /// Add request id header and extension, using `x-request-id` as the header name.
    ///
    /// See [`tower_async_http::request_id`] for more details.
    ///
    /// [`tower_async_http::request_id`]: crate::service::http::request_id
    pub fn set_x_request_id<M>(
        self,
        make_request_id: M,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::request_id::SetRequestIdLayer<M>, L>,
    >
    where
        M: crate::service::http::request_id::MakeRequestId,
    {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.set_x_request_id(make_request_id),
        }
    }

    /// Propgate request ids from requests to responses.
    ///
    /// See [`tower_async_http::request_id`] for more details.
    ///
    /// [`tower_async_http::request_id`]: crate::service::http::request_id
    pub fn propagate_request_id(
        self,
        header_name: crate::http::HeaderName,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::request_id::PropagateRequestIdLayer, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.propagate_request_id(header_name),
        }
    }

    /// Propgate request ids from requests to responses, using `x-request-id` as the header name.
    ///
    /// See [`tower_async_http::request_id`] for more details.
    ///
    /// [`tower_async_http::request_id`]: crate::service::http::request_id
    pub fn propagate_x_request_id(
        self,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::request_id::PropagateRequestIdLayer, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.propagate_x_request_id(),
        }
    }

    /// Catch panics and convert them into `500 Internal Server` responses.
    ///
    /// See [`tower_async_http::catch_panic`] for more details.
    ///
    /// [`tower_async_http::catch_panic`]: crate::service::http::catch_panic
    pub fn catch_panic(
        self,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<
            crate::service::http::catch_panic::CatchPanicLayer<
                crate::service::http::catch_panic::DefaultResponseForPanic,
            >,
            L,
        >,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.catch_panic(),
        }
    }

    /// Intercept requests with over-sized payloads and convert them into
    /// `413 Payload Too Large` responses.
    ///
    /// See [`tower_async_http::limit`] for more details.
    ///
    /// [`tower_async_http::limit`]: crate::service::http::limit
    pub fn request_body_limit(
        self,
        limit: usize,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::limit::RequestBodyLimitLayer, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.request_body_limit(limit),
        }
    }

    /// Remove trailing slashes from paths.
    ///
    /// See [`tower_async_http::normalize_path`] for more details.
    ///
    /// [`tower_async_http::normalize_path`]: crate::service::http::normalize_path
    pub fn trim_trailing_slash(
        self,
    ) -> HttpServer<
        B,
        crate::service::util::Stack<crate::service::http::normalize_path::NormalizePathLayer, L>,
    > {
        HttpServer {
            builder: self.builder,
            service_builder: self.service_builder.trim_trailing_slash(),
        }
    }
}

impl<B, L> HttpServer<B, L>
where
    B: HyperConnServer,
{
    pub fn service<TowerService, ResponseBody, D, E>(
        self,
        service: TowerService,
    ) -> HttpService<B, L::Service>
    where
        L: Layer<TowerService>,
        L::Service: Service<Request, Response = Response<ResponseBody>, call(): Send>
            + Send
            + Sync
            + 'static,
        <L::Service as Service<Request>>::Error: Into<BoxError>,
        TowerService: Service<Request, call(): Send> + Send + Sync + 'static,
        TowerService::Error: Into<BoxError>,
        ResponseBody: http_body::Body<Data = D, Error = E> + Send + 'static,
        D: Send,
        E: Into<BoxError>,
    {
        let service = self.service_builder.service(service);
        HttpService::new(self.builder, service)
    }

    pub async fn serve<S, TowerService, ResponseBody, D, E>(
        &self,
        stream: TcpStream<S>,
        service: TowerService,
    ) -> ServeResult
    where
        S: crate::stream::Stream + Send + 'static,
        L: Layer<TowerService>,
        L::Service: Service<Request, Response = Response<ResponseBody>, call(): Send>
            + Send
            + Sync
            + 'static,
        <L::Service as Service<Request>>::Error: Into<BoxError>,
        TowerService: Service<Request, call(): Send> + Send + Sync + 'static,
        TowerService::Error: Into<BoxError>,
        ResponseBody: http_body::Body<Data = D, Error = E> + Send + 'static,
        D: Send,
        E: Into<BoxError>,
    {
        let service = self.service_builder.service(service);
        let service: HyperServiceWrapper<<L as Layer<TowerService>>::Service> =
            HyperServiceWrapper {
                service: Arc::new(service),
            };

        self.builder.hyper_serve_connection(stream, service).await
    }
}

pub struct HttpService<B, S> {
    builder: Arc<B>,
    service: HyperServiceWrapper<S>,
}

impl<B, S> std::fmt::Debug for HttpService<B, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpService").finish()
    }
}

impl<B, S> HttpService<B, S> {
    fn new(builder: B, service: S) -> Self {
        Self {
            builder: Arc::new(builder),
            service: HyperServiceWrapper {
                service: Arc::new(service),
            },
        }
    }
}

impl<B, S> Clone for HttpService<B, S> {
    fn clone(&self) -> Self {
        Self {
            builder: self.builder.clone(),
            service: self.service.clone(),
        }
    }
}

impl<B, T, S, Body> Service<TcpStream<T>> for HttpService<B, S>
where
    B: HyperConnServer,
    T: crate::stream::Stream + Send + 'static,
    S: Service<Request, call(): Send, Response = Response<Body>> + Send + Sync + 'static,
    S::Error: Into<BoxError>,
    Body: http_body::Body + Send + 'static,
    Body::Data: Send,
    Body::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Response = ();
    type Error = BoxError;

    async fn call(&self, stream: TcpStream<T>) -> Result<Self::Response, Self::Error> {
        let service = self.service.clone();
        self.builder.hyper_serve_connection(stream, service).await
    }
}

#[derive(Debug)]
struct HyperServiceWrapper<S> {
    service: Arc<S>,
}

impl<S> Clone for HyperServiceWrapper<S> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
        }
    }
}

impl<S> hyper::service::Service<crate::http::Request<hyper::body::Incoming>>
    for HyperServiceWrapper<S>
where
    S: Service<Request, call(): Send> + Send + Sync + 'static,
    Request: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn call(&self, req: crate::http::Request<hyper::body::Incoming>) -> Self::Future {
        let (parts, body) = req.into_parts();
        let req = Request::from_parts(parts, HyperBody::from(body));

        let service = self.service.clone();
        let fut = async move { service.call(req).await };
        Box::pin(fut)
    }
}

pub type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
