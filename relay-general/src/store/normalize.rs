use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::mem;
use std::sync::Arc;

use chrono::{Duration, Utc};
use itertools::Itertools;
use lazy_static::lazy_static;
use regex::Regex;
use smallvec::SmallVec;

use crate::processor::{MaxChars, ProcessValue, ProcessingState, Processor};
use crate::protocol::{
    AsPair, Breadcrumb, ClientSdkInfo, Context, DebugImage, Event, EventId, EventType, Exception,
    Frame, HeaderName, HeaderValue, Headers, IpAddr, Level, LogEntry, Request, SpanStatus,
    Stacktrace, Tags, TraceContext, User, INVALID_ENVIRONMENTS, INVALID_RELEASES, VALID_PLATFORMS,
};
use crate::store::{GeoIpLookup, StoreConfig};
use crate::types::{
    Annotated, Empty, Error, ErrorKind, FromValue, Meta, Object, ProcessingAction,
    ProcessingResult, Value,
};

mod contexts;
mod logentry;
mod mechanism;
mod request;
mod stacktrace;

#[cfg(feature = "uaparser")]
mod user_agent;

/// Validate fields that go into a `sentry.models.BoundedIntegerField`.
fn validate_bounded_integer_field(value: u64) -> ProcessingResult {
    if value < 2_147_483_647 {
        Ok(())
    } else {
        Err(ProcessingAction::DeleteValueHard)
    }
}

struct DedupCache(SmallVec<[u64; 16]>);

impl DedupCache {
    pub fn new() -> Self {
        Self(SmallVec::default())
    }

    pub fn probe<H: Hash>(&mut self, element: H) -> bool {
        let mut hasher = DefaultHasher::new();
        element.hash(&mut hasher);
        let hash = hasher.finish();

        if self.0.contains(&hash) {
            false
        } else {
            self.0.push(hash);
            true
        }
    }
}

pub fn is_valid_platform(platform: &str) -> bool {
    VALID_PLATFORMS.contains(&platform)
}

pub fn is_valid_environment(environment: &str) -> bool {
    !INVALID_ENVIRONMENTS.contains(&environment)
}

pub fn is_valid_release(release: &str) -> bool {
    !INVALID_RELEASES
        .iter()
        .any(|invalid| release.eq_ignore_ascii_case(invalid))
}

/// The processor that normalizes events for store.
pub struct NormalizeProcessor<'a> {
    config: Arc<StoreConfig>,
    geoip_lookup: Option<&'a GeoIpLookup>,
}

impl<'a> NormalizeProcessor<'a> {
    /// Creates a new normalization processor.
    pub fn new(config: Arc<StoreConfig>, geoip_lookup: Option<&'a GeoIpLookup>) -> Self {
        NormalizeProcessor {
            config,
            geoip_lookup,
        }
    }

    /// Returns the SDK info from the config.
    fn get_sdk_info(&self) -> Option<ClientSdkInfo> {
        self.config.client.as_ref().and_then(|client| {
            client
                .splitn(2, '/')
                .collect_tuple()
                .or_else(|| client.splitn(2, ' ').collect_tuple())
                .map(|(name, version)| ClientSdkInfo {
                    name: Annotated::new(name.to_owned()),
                    version: Annotated::new(version.to_owned()),
                    ..Default::default()
                })
        })
    }

    /// Ensures that the `release` and `dist` fields match up.
    fn normalize_release_dist(&self, event: &mut Event) {
        if event.dist.value().is_some() && event.release.value().is_empty() {
            event.dist.set_value(None);
        }

        if let Some(dist) = event.dist.value_mut() {
            if dist.trim() != dist {
                *dist = dist.trim().to_string();
            }
        }
    }

    /// Validates the timestamp range and sets a default value.
    fn normalize_timestamps(&self, event: &mut Event) -> ProcessingResult {
        let current_timestamp = Utc::now();
        event.received = Annotated::new(current_timestamp);

        event.timestamp.apply(|timestamp, meta| {
            if let Some(secs) = self.config.max_secs_in_future {
                if *timestamp > current_timestamp + Duration::seconds(secs) {
                    meta.add_error(ErrorKind::FutureTimestamp);
                    return Err(ProcessingAction::DeleteValueSoft);
                }
            }

            if let Some(secs) = self.config.max_secs_in_past {
                if *timestamp < current_timestamp - Duration::seconds(secs) {
                    meta.add_error(ErrorKind::PastTimestamp);
                    return Err(ProcessingAction::DeleteValueSoft);
                }
            }

            Ok(())
        })?;

        if event.timestamp.value().is_none() {
            event.timestamp.set_value(Some(current_timestamp));
        }

        event
            .time_spent
            .apply(|time_spent, _| validate_bounded_integer_field(*time_spent))?;

        Ok(())
    }

    /// Removes internal tags and adds tags for well-known attributes.
    fn normalize_event_tags(&self, event: &mut Event) -> ProcessingResult {
        let tags = &mut event.tags.value_mut().get_or_insert_with(Tags::default).0;
        let environment = &mut event.environment;
        if environment.is_empty() {
            *environment = Annotated::empty();
        }

        // Fix case where legacy apps pass environment as a tag instead of a top level key
        if let Some(tag) = tags.remove("environment").and_then(Annotated::into_value) {
            environment.get_or_insert_with(|| tag);
        }

        // Remove internal tags, that are generated with a `sentry:` prefix when saving the event.
        // They are not allowed to be set by the client due to ambiguity. Also, deduplicate tags.
        let mut tag_cache = DedupCache::new();
        tags.retain(|entry| {
            match entry.value() {
                Some(tag) => match tag.key().unwrap_or_default() {
                    "" | "release" | "dist" | "user" | "filename" | "function" => false,
                    name => tag_cache.probe(name),
                },
                // ToValue will decide if we should skip serializing Annotated::empty()
                None => true,
            }
        });

        for tag in tags.iter_mut() {
            tag.apply(|tag, meta| {
                if let Some(key) = tag.key() {
                    if bytecount::num_chars(key.as_bytes()) > MaxChars::TagKey.limit() {
                        meta.add_error(Error::new(ErrorKind::ValueTooLong));
                        return Err(ProcessingAction::DeleteValueHard);
                    }
                }

                if let Some(value) = tag.value() {
                    if value.is_empty() {
                        meta.add_error(Error::nonempty());
                        return Err(ProcessingAction::DeleteValueHard);
                    }

                    if bytecount::num_chars(value.as_bytes()) > MaxChars::TagValue.limit() {
                        meta.add_error(Error::new(ErrorKind::ValueTooLong));
                        return Err(ProcessingAction::DeleteValueHard);
                    }
                }

                Ok(())
            })?;
        }

        let server_name = std::mem::take(&mut event.server_name);
        if server_name.value().is_some() {
            tags.insert("server_name".to_string(), server_name);
        }

        let site = std::mem::take(&mut event.site);
        if site.value().is_some() {
            tags.insert("site".to_string(), site);
        }

        Ok(())
    }

    /// Infers the `EventType` from the event's interfaces.
    fn infer_event_type(&self, event: &Event) -> EventType {
        if let Some(ty) = event.ty.value() {
            return *ty;
        }

        // port of src/sentry/eventtypes
        //
        // the SDKs currently do not describe event types, and we must infer
        // them from available attributes

        let has_exceptions = event
            .exceptions
            .value()
            .and_then(|exceptions| exceptions.values.value())
            .filter(|values| !values.is_empty())
            .is_some();

        if has_exceptions {
            EventType::Error
        } else if event.csp.value().is_some() {
            EventType::Csp
        } else if event.hpkp.value().is_some() {
            EventType::Hpkp
        } else if event.expectct.value().is_some() {
            EventType::ExpectCT
        } else if event.expectstaple.value().is_some() {
            EventType::ExpectStaple
        } else {
            EventType::Default
        }
    }

    fn is_security_report(&self, event: &Event) -> bool {
        event.csp.value().is_some()
            || event.expectct.value().is_some()
            || event.expectstaple.value().is_some()
            || event.hpkp.value().is_some()
    }

    /// Backfills common security report attributes.
    fn normalize_security_report(&self, event: &mut Event) {
        if !self.is_security_report(event) {
            // This event is not a security report, exit here.
            return;
        }

        event.logger.get_or_insert_with(|| "csp".to_string());

        if let Some(ref client_ip) = self.config.client_ip {
            let user = event.user.value_mut().get_or_insert_with(User::default);
            user.ip_address = Annotated::new(client_ip.clone());
        }

        if let Some(ref client) = self.config.user_agent {
            let request = event
                .request
                .value_mut()
                .get_or_insert_with(Request::default);

            let headers = request
                .headers
                .value_mut()
                .get_or_insert_with(Headers::default);

            if !headers.contains("User-Agent") {
                headers.insert(
                    HeaderName::new("User-Agent".to_owned()),
                    Annotated::new(HeaderValue::new(client.clone())),
                );
            }
        }
    }

    /// Backfills IP addresses in various places.
    fn normalize_ip_addresses(&self, event: &mut Event) {
        // NOTE: This is highly order dependent, in the sense that both the statements within this
        // function need to be executed in a certain order, and that other normalization code
        // (geoip lookup) needs to run after this.
        //
        // After a series of regressions over the old Python spaghetti code we decided to put it
        // back into one function. If a desire to split this code up overcomes you, put this in a
        // new processor and make sure all of it runs before the rest of normalization.

        // Resolve {{auto}}
        if let Some(ref client_ip) = self.config.client_ip {
            if let Some(ref mut request) = event.request.value_mut() {
                if let Some(ref mut env) = request.env.value_mut() {
                    if let Some(&mut Value::String(ref mut http_ip)) = env
                        .get_mut("REMOTE_ADDR")
                        .and_then(|annotated| annotated.value_mut().as_mut())
                    {
                        if http_ip == "{{auto}}" {
                            *http_ip = client_ip.to_string();
                        }
                    }
                }
            }

            if let Some(ref mut user) = event.user.value_mut() {
                if let Some(ref mut user_ip) = user.ip_address.value_mut() {
                    if user_ip.is_auto() {
                        *user_ip = client_ip.clone();
                    }
                }
            }
        }

        // Copy IPs from request interface to user, and resolve platform-specific backfilling
        let http_ip = event
            .request
            .value()
            .and_then(|request| request.env.value())
            .and_then(|env| env.get("REMOTE_ADDR"))
            .and_then(Annotated::<Value>::as_str)
            .and_then(|ip| IpAddr::parse(ip).ok());

        if let Some(http_ip) = http_ip {
            let user = event.user.value_mut().get_or_insert_with(User::default);
            user.ip_address.value_mut().get_or_insert(http_ip);
        } else if let Some(ref client_ip) = self.config.client_ip {
            let user = event.user.value_mut().get_or_insert_with(User::default);
            // auto is already handled above
            if user.ip_address.value().is_none() {
                let platform = event.platform.as_str();

                // In an ideal world all SDKs would set {{auto}} explicitly.
                if let Some("javascript") | Some("cocoa") | Some("objc") = platform {
                    user.ip_address = Annotated::new(client_ip.clone());
                }
            }
        }
    }

    fn normalize_exceptions(&self, event: &mut Event) -> ProcessingResult {
        let os_hint = mechanism::OsHint::from_event(&event);

        if let Some(exception_values) = event.exceptions.value_mut() {
            if let Some(exceptions) = exception_values.values.value_mut() {
                if exceptions.len() == 1 && event.stacktrace.value().is_some() {
                    if let Some(exception) = exceptions.get_mut(0) {
                        if let Some(exception) = exception.value_mut() {
                            mem::swap(&mut exception.stacktrace, &mut event.stacktrace);
                            event.stacktrace = Annotated::empty();
                        }
                    }
                }

                // Exception mechanism needs SDK information to resolve proper names in
                // exception meta (such as signal names). "SDK Information" really means
                // the operating system version the event was generated on. Some
                // normalization still works without sdk_info, such as mach_exception
                // names (they can only occur on macOS).
                //
                // We also want to validate some other aspects of it.
                for exception in exceptions {
                    if let Some(exception) = exception.value_mut() {
                        if let Some(mechanism) = exception.mechanism.value_mut() {
                            mechanism::normalize_mechanism(mechanism, os_hint)?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn normalize_user_agent(&self, _event: &mut Event) {
        if self.config.normalize_user_agent.unwrap_or(false) {
            #[cfg(feature = "uaparser")]
            user_agent::normalize_user_agent(_event);

            #[cfg(not(feature = "uaparser"))]
            panic!("relay not built with uaparser feature");
        }
    }
}

impl<'a> Processor for NormalizeProcessor<'a> {
    fn process_event(
        &mut self,
        event: &mut Event,
        _meta: &mut Meta,
        state: &ProcessingState<'_>,
    ) -> ProcessingResult {
        // Process security reports first to ensure all props.
        self.normalize_security_report(event);

        // Insert IP addrs before recursing, since geo lookup depends on it.
        self.normalize_ip_addresses(event);

        event.process_child_values(self, state)?;

        // Override internal attributes, even if they were set in the payload
        event.project = Annotated::from(self.config.project_id);
        event.key_id = Annotated::from(self.config.key_id.clone());
        event.ty = Annotated::from(self.infer_event_type(event));
        event.version = Annotated::from(self.config.protocol_version.clone());
        event.grouping_config = self
            .config
            .grouping_config
            .clone()
            .map_or(Annotated::empty(), |x| {
                FromValue::from_value(Annotated::<Value>::from(x))
            });

        // Validate basic attributes
        event.platform.apply(|platform, _| {
            if is_valid_platform(&platform) {
                Ok(())
            } else {
                Err(ProcessingAction::DeleteValueSoft)
            }
        })?;

        event.environment.apply(|environment, meta| {
            if is_valid_environment(&environment) {
                Ok(())
            } else {
                meta.add_error(ErrorKind::InvalidData);
                Err(ProcessingAction::DeleteValueSoft)
            }
        })?;

        event.release.apply(|release, meta| {
            if is_valid_release(&release) {
                Ok(())
            } else {
                meta.add_error(ErrorKind::InvalidData);
                Err(ProcessingAction::DeleteValueSoft)
            }
        })?;

        // Default required attributes, even if they have errors
        event.errors.get_or_insert_with(Vec::new);
        event.level.get_or_insert_with(|| Level::Error);
        event.id.get_or_insert_with(EventId::new);
        event.platform.get_or_insert_with(|| "other".to_string());
        event.logger.get_or_insert_with(String::new);
        event.extra.get_or_insert_with(Object::new);
        if event.client_sdk.value().is_none() {
            event.client_sdk.set_value(self.get_sdk_info());
        }

        // Normalize connected attributes and interfaces
        self.normalize_release_dist(event);
        self.normalize_timestamps(event)?;
        self.normalize_event_tags(event)?;
        self.normalize_exceptions(event)?;
        self.normalize_user_agent(event);

        Ok(())
    }

    fn process_breadcrumb(
        &mut self,
        breadcrumb: &mut Breadcrumb,
        _meta: &mut Meta,
        state: &ProcessingState<'_>,
    ) -> ProcessingResult {
        breadcrumb.process_child_values(self, state)?;

        if breadcrumb.ty.value().is_empty() {
            breadcrumb.ty.set_value(Some("default".to_string()));
        }

        if breadcrumb.level.value().is_none() {
            breadcrumb.level.set_value(Some(Level::Info));
        }

        Ok(())
    }

    fn process_request(
        &mut self,
        request: &mut Request,
        _meta: &mut Meta,
        state: &ProcessingState<'_>,
    ) -> ProcessingResult {
        request.process_child_values(self, state)?;

        request::normalize_request(request)?;

        Ok(())
    }

    fn process_user(
        &mut self,
        user: &mut User,
        _meta: &mut Meta,
        state: &ProcessingState<'_>,
    ) -> ProcessingResult {
        if !user.other.is_empty() {
            let data = user.data.value_mut().get_or_insert_with(Object::new);
            data.extend(std::mem::take(&mut user.other).into_iter());
        }

        user.process_child_values(self, state)?;

        // Infer user.geo from user.ip_address
        if user.geo.value().is_none() {
            if let Some(ref geoip_lookup) = self.geoip_lookup {
                if let Some(ip_address) = user.ip_address.value() {
                    if let Ok(Some(geo)) = geoip_lookup.lookup(ip_address.as_str()) {
                        user.geo.set_value(Some(geo));
                    }
                }
            }
        }

        Ok(())
    }

    fn process_debug_image(
        &mut self,
        image: &mut DebugImage,
        meta: &mut Meta,
        _state: &ProcessingState<'_>,
    ) -> ProcessingResult {
        match image {
            DebugImage::Other(_) => {
                meta.add_error(Error::invalid("unsupported debug image type"));
                Err(ProcessingAction::DeleteValueSoft)
            }
            _ => Ok(()),
        }
    }

    fn process_logentry(
        &mut self,
        logentry: &mut LogEntry,
        meta: &mut Meta,
        _state: &ProcessingState<'_>,
    ) -> ProcessingResult {
        logentry::normalize_logentry(logentry, meta)
    }

    fn process_exception(
        &mut self,
        exception: &mut Exception,
        meta: &mut Meta,
        state: &ProcessingState<'_>,
    ) -> ProcessingResult {
        exception.process_child_values(self, state)?;

        lazy_static! {
            static ref TYPE_VALUE_RE: Regex = Regex::new(r"^(\w+):(.*)$").unwrap();
        }

        if exception.ty.value().is_empty() {
            if let Some(value_str) = exception.value.value_mut() {
                let new_values = TYPE_VALUE_RE
                    .captures(value_str)
                    .map(|cap| (cap[1].to_string(), cap[2].trim().to_string().into()));

                if let Some((new_type, new_value)) = new_values {
                    exception.ty.set_value(Some(new_type));
                    *value_str = new_value;
                }
            }
        }

        if exception.ty.value().is_empty() && exception.value.value().is_empty() {
            meta.add_error(Error::with(ErrorKind::MissingAttribute, |error| {
                error.insert("attribute", "type or value");
            }));
            return Err(ProcessingAction::DeleteValueSoft);
        }

        Ok(())
    }

    fn process_frame(
        &mut self,
        frame: &mut Frame,
        _meta: &mut Meta,
        state: &ProcessingState<'_>,
    ) -> ProcessingResult {
        frame.process_child_values(self, state)?;

        if frame.function.as_str() == Some("?") {
            frame.function.set_value(None);
        }

        if frame.symbol.as_str() == Some("?") {
            frame.symbol.set_value(None);
        }

        if let Some(lines) = frame.pre_context.value_mut() {
            for line in lines.iter_mut() {
                line.get_or_insert_with(String::new);
            }
        }

        if let Some(lines) = frame.post_context.value_mut() {
            for line in lines.iter_mut() {
                line.get_or_insert_with(String::new);
            }
        }

        if frame.context_line.value().is_none()
            && (!frame.pre_context.is_empty() || !frame.post_context.is_empty())
        {
            frame.context_line.set_value(Some(String::new()));
        }

        Ok(())
    }

    fn process_stacktrace(
        &mut self,
        stacktrace: &mut Stacktrace,
        meta: &mut Meta,
        _state: &ProcessingState<'_>,
    ) -> ProcessingResult {
        stacktrace::process_stacktrace(&mut stacktrace.0, meta)?;
        Ok(())
    }

    fn process_context(
        &mut self,
        context: &mut Context,
        _meta: &mut Meta,
        _state: &ProcessingState<'_>,
    ) -> ProcessingResult {
        contexts::normalize_context(context);
        Ok(())
    }

    fn process_trace_context(
        &mut self,
        context: &mut TraceContext,
        _meta: &mut Meta,
        _state: &ProcessingState<'_>,
    ) -> ProcessingResult {
        context
            .status
            .value_mut()
            .get_or_insert(SpanStatus::UnknownError);
        Ok(())
    }
}

#[cfg(test)]
use crate::{
    processor::process_value,
    protocol::{PairList, TagEntry},
};

#[cfg(test)]
impl Default for NormalizeProcessor<'_> {
    fn default() -> Self {
        NormalizeProcessor::new(Arc::new(StoreConfig::default()), None)
    }
}

#[test]
fn test_handles_type_in_value() {
    let mut processor = NormalizeProcessor::default();

    let mut exception = Annotated::new(Exception {
        value: Annotated::new("ValueError: unauthorized".to_string().into()),
        ..Exception::default()
    });

    process_value(&mut exception, &mut processor, ProcessingState::root()).unwrap();
    let exception = exception.value().unwrap();
    assert_eq_dbg!(exception.value.as_str(), Some("unauthorized"));
    assert_eq_dbg!(exception.ty.as_str(), Some("ValueError"));

    let mut exception = Annotated::new(Exception {
        value: Annotated::new("ValueError:unauthorized".to_string().into()),
        ..Exception::default()
    });

    process_value(&mut exception, &mut processor, ProcessingState::root()).unwrap();
    let exception = exception.value().unwrap();
    assert_eq_dbg!(exception.value.as_str(), Some("unauthorized"));
    assert_eq_dbg!(exception.ty.as_str(), Some("ValueError"));
}

#[test]
fn test_rejects_empty_exception_fields() {
    let mut processor = NormalizeProcessor::new(Arc::new(StoreConfig::default()), None);

    let mut exception = Annotated::new(Exception {
        value: Annotated::new("".to_string().into()),
        ty: Annotated::new("".to_string()),
        ..Default::default()
    });

    process_value(&mut exception, &mut processor, ProcessingState::root()).unwrap();
    assert!(exception.value().is_none());
    assert!(exception.meta().has_errors());
}

#[test]
fn test_json_value() {
    let mut processor = NormalizeProcessor::default();

    let mut exception = Annotated::new(Exception {
        value: Annotated::new(r#"{"unauthorized":true}"#.to_string().into()),
        ..Exception::default()
    });
    process_value(&mut exception, &mut processor, ProcessingState::root()).unwrap();
    let exception = exception.value().unwrap();

    // Don't split a json-serialized value on the colon
    assert_eq_dbg!(exception.value.as_str(), Some(r#"{"unauthorized":true}"#));
    assert_eq_dbg!(exception.ty.value(), None);
}

#[test]
fn test_exception_invalid() {
    let mut processor = NormalizeProcessor::default();

    let mut exception = Annotated::new(Exception::default());
    process_value(&mut exception, &mut processor, ProcessingState::root()).unwrap();

    let expected = Error::with(ErrorKind::MissingAttribute, |error| {
        error.insert("attribute", "type or value");
    });

    assert_eq_dbg!(
        exception.meta().iter_errors().collect_tuple(),
        Some((&expected,))
    );
}

#[test]
fn test_geo_from_ip_address() {
    use crate::protocol::Geo;

    let lookup = GeoIpLookup::open("../GeoLite2-City.mmdb").unwrap();
    let mut processor = NormalizeProcessor::new(Arc::new(StoreConfig::default()), Some(&lookup));

    let mut user = Annotated::new(User {
        ip_address: Annotated::new(IpAddr("213.47.147.207".to_string())),
        ..User::default()
    });

    process_value(&mut user, &mut processor, ProcessingState::root()).unwrap();

    let expected = Annotated::new(Geo {
        country_code: Annotated::new("AT".to_string()),
        city: Annotated::new("Vienna".to_string()),
        region: Annotated::new("Austria".to_string()),
        ..Geo::default()
    });
    assert_eq_dbg!(user.value().unwrap().geo, expected)
}

#[test]
fn test_user_ip_from_remote_addr() {
    let mut event = Annotated::new(Event {
        request: Annotated::from(Request {
            env: Annotated::new({
                let mut map = Object::new();
                map.insert(
                    "REMOTE_ADDR".to_string(),
                    Annotated::new(Value::String("213.47.147.207".to_string())),
                );
                map
            }),
            ..Request::default()
        }),
        platform: Annotated::new("javascript".to_owned()),
        ..Event::default()
    });

    let config = StoreConfig::default();
    let mut processor = NormalizeProcessor::new(Arc::new(config), None);
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let user = event
        .value()
        .unwrap()
        .user
        .value()
        .expect("user was not created");

    let ip_addr = user.ip_address.value().expect("ip address was not created");

    assert_eq_dbg!(ip_addr, &IpAddr("213.47.147.207".to_string()));
}

#[test]
fn test_user_ip_from_invalid_remote_addr() {
    let mut event = Annotated::new(Event {
        request: Annotated::from(Request {
            env: Annotated::new({
                let mut map = Object::new();
                map.insert(
                    "REMOTE_ADDR".to_string(),
                    Annotated::new(Value::String("whoops".to_string())),
                );
                map
            }),
            ..Request::default()
        }),
        platform: Annotated::new("javascript".to_owned()),
        ..Event::default()
    });

    let config = StoreConfig::default();
    let mut processor = NormalizeProcessor::new(Arc::new(config), None);
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    assert_eq_dbg!(Annotated::empty(), event.value().unwrap().user);
}

#[test]
fn test_user_ip_from_client_ip_without_auto() {
    let mut event = Annotated::new(Event {
        platform: Annotated::new("javascript".to_owned()),
        ..Default::default()
    });

    let mut config = StoreConfig::default();
    config.client_ip = Some(IpAddr::parse("213.47.147.207").unwrap());

    let mut processor = NormalizeProcessor::new(Arc::new(config), None);
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let user = event
        .value()
        .unwrap()
        .user
        .value()
        .expect("user was not created");

    let ip_addr = user.ip_address.value().expect("ip address was not created");

    assert_eq_dbg!(ip_addr, &IpAddr("213.47.147.207".to_string()));
}

#[test]
fn test_user_ip_from_client_ip_with_auto() {
    let mut event = Annotated::new(Event {
        user: Annotated::new(User {
            ip_address: Annotated::new(IpAddr::auto()),
            ..Default::default()
        }),
        ..Default::default()
    });

    let mut config = StoreConfig::default();
    config.client_ip = Some(IpAddr::parse("213.47.147.207").unwrap());

    let geo = GeoIpLookup::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../GeoLite2-City.mmdb"
    ))
    .unwrap();
    let mut processor = NormalizeProcessor::new(Arc::new(config), Some(&geo));
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let user = event.value().unwrap().user.value().expect("user missing");

    let ip_addr = user.ip_address.value().expect("ip address missing");

    assert_eq_dbg!(ip_addr, &IpAddr("213.47.147.207".to_string()));
    assert!(user.geo.value().is_some());
}

#[test]
fn test_user_ip_from_client_ip_without_appropriate_platform() {
    let mut event = Annotated::new(Event::default());

    let mut config = StoreConfig::default();
    config.client_ip = Some(IpAddr::parse("213.47.147.207").unwrap());

    let geo = GeoIpLookup::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../GeoLite2-City.mmdb"
    ))
    .unwrap();
    let mut processor = NormalizeProcessor::new(Arc::new(config), Some(&geo));
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let user = event.value().unwrap().user.value().expect("user missing");

    assert!(user.ip_address.value().is_none());
    assert!(user.geo.value().is_none());
}

#[test]
fn test_environment_tag_is_moved() {
    let mut event = Annotated::new(Event {
        tags: Annotated::new(Tags(PairList(vec![Annotated::new(TagEntry(
            Annotated::new("environment".to_string()),
            Annotated::new("despacito".to_string()),
        ))]))),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let event = event.value().unwrap();

    assert_eq_dbg!(event.environment.as_str(), Some("despacito"));
    assert_eq_dbg!(event.tags.value(), Some(&Tags(vec![].into())));
}

#[test]
fn test_empty_environment_is_removed_and_overwritten_with_tag() {
    let mut event = Annotated::new(Event {
        tags: Annotated::new(Tags(PairList(vec![Annotated::new(TagEntry(
            Annotated::new("environment".to_string()),
            Annotated::new("despacito".to_string()),
        ))]))),
        environment: Annotated::new("".to_string()),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let event = event.value().unwrap();

    assert_eq_dbg!(event.environment.as_str(), Some("despacito"));
    assert_eq_dbg!(event.tags.value(), Some(&Tags(vec![].into())));
}

#[test]
fn test_empty_environment_is_removed() {
    let mut event = Annotated::new(Event {
        environment: Annotated::new("".to_string()),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let event = event.value().unwrap();
    assert_eq_dbg!(event.environment.value(), None);
}

#[test]
fn test_none_environment_errors() {
    let mut event = Annotated::new(Event {
        environment: Annotated::new("none".to_string()),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let environment = &event.value().unwrap().environment;
    let expected_original = &Value::String("none".to_string());

    assert_eq_dbg!(
        environment.meta().iter_errors().collect::<Vec<&Error>>(),
        vec![&Error::new(ErrorKind::InvalidData)],
    );
    assert_eq_dbg!(
        environment.meta().original_value().unwrap(),
        expected_original
    );
    assert_eq_dbg!(environment.value(), None);
}

#[test]
fn test_invalid_release_removed() {
    use crate::protocol::LenientString;

    let mut event = Annotated::new(Event {
        release: Annotated::new(LenientString("Latest".to_string())),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let release = &event.value().unwrap().release;
    let expected_original = &Value::String("Latest".to_string());

    assert_eq_dbg!(
        release.meta().iter_errors().collect::<Vec<&Error>>(),
        vec![&Error::new(ErrorKind::InvalidData)],
    );
    assert_eq_dbg!(release.meta().original_value().unwrap(), expected_original);
    assert_eq_dbg!(release.value(), None);
}

#[test]
fn test_top_level_keys_moved_into_tags() {
    let mut event = Annotated::new(Event {
        server_name: Annotated::new("foo".to_string()),
        site: Annotated::new("foo".to_string()),
        tags: Annotated::new(Tags(PairList(vec![
            Annotated::new(TagEntry(
                Annotated::new("site".to_string()),
                Annotated::new("old".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("server_name".to_string()),
                Annotated::new("old".to_string()),
            )),
        ]))),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let event = event.value().unwrap();

    assert_eq_dbg!(event.site.value(), None);
    assert_eq_dbg!(event.server_name.value(), None);

    assert_eq_dbg!(
        event.tags.value(),
        Some(&Tags(PairList(vec![
            Annotated::new(TagEntry(
                Annotated::new("site".to_string()),
                Annotated::new("foo".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("server_name".to_string()),
                Annotated::new("foo".to_string()),
            )),
        ])))
    );
}

#[test]
fn test_internal_tags_removed() {
    let mut event = Annotated::new(Event {
        tags: Annotated::new(Tags(PairList(vec![
            Annotated::new(TagEntry(
                Annotated::new("release".to_string()),
                Annotated::new("foo".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("dist".to_string()),
                Annotated::new("foo".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("user".to_string()),
                Annotated::new("foo".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("filename".to_string()),
                Annotated::new("foo".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("function".to_string()),
                Annotated::new("foo".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("something".to_string()),
                Annotated::new("else".to_string()),
            )),
        ]))),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    assert_eq!(event.value().unwrap().tags.value().unwrap().len(), 1);
}

#[test]
fn test_empty_tags_removed() {
    let mut event = Annotated::new(Event {
        tags: Annotated::new(Tags(PairList(vec![
            Annotated::new(TagEntry(
                Annotated::new("".to_string()),
                Annotated::new("foo".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("foo".to_string()),
                Annotated::new("".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("something".to_string()),
                Annotated::new("else".to_string()),
            )),
        ]))),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let tags = event.value().unwrap().tags.value().unwrap();
    assert_eq!(tags.len(), 2);

    assert_eq!(
        tags.get(0).unwrap(),
        &Annotated::from_error(Error::nonempty(), None)
    )
}

#[test]
fn test_tags_deduplicated() {
    let mut event = Annotated::new(Event {
        tags: Annotated::new(Tags(PairList(vec![
            Annotated::new(TagEntry(
                Annotated::new("foo".to_string()),
                Annotated::new("1".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("bar".to_string()),
                Annotated::new("1".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("foo".to_string()),
                Annotated::new("2".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("bar".to_string()),
                Annotated::new("2".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("foo".to_string()),
                Annotated::new("3".to_string()),
            )),
        ]))),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    // should keep the first occurrence of every tag
    assert_eq_dbg!(
        event.value().unwrap().tags.value().unwrap(),
        &Tags(PairList(vec![
            Annotated::new(TagEntry(
                Annotated::new("foo".to_string()),
                Annotated::new("1".to_string()),
            )),
            Annotated::new(TagEntry(
                Annotated::new("bar".to_string()),
                Annotated::new("1".to_string()),
            )),
        ]))
    );
}

#[test]
fn test_user_data_moved() {
    let mut user = Annotated::new(User {
        other: {
            let mut map = Object::new();
            map.insert(
                "other".to_string(),
                Annotated::new(Value::String("value".to_owned())),
            );
            map
        },
        ..User::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut user, &mut processor, ProcessingState::root()).unwrap();

    let user = user.value().unwrap();

    assert_eq_dbg!(user.data, {
        let mut map = Object::new();
        map.insert(
            "other".to_string(),
            Annotated::new(Value::String("value".to_owned())),
        );
        Annotated::new(map)
    });

    assert_eq_dbg!(user.other, Object::new());
}

#[test]
fn test_unknown_debug_image() {
    use crate::protocol::DebugMeta;

    let mut event = Annotated::new(Event {
        debug_meta: Annotated::new(DebugMeta {
            images: Annotated::new(vec![Annotated::new(DebugImage::Other(Object::default()))]),
            ..DebugMeta::default()
        }),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let expected = Annotated::new(DebugMeta {
        images: Annotated::new(vec![Annotated::from_error(
            Error::invalid("unsupported debug image type"),
            Some(Value::Object(Object::default())),
        )]),
        ..DebugMeta::default()
    });

    assert_eq_dbg!(expected, event.value().unwrap().debug_meta);
}

#[test]
fn test_context_line_default() {
    let mut frame = Annotated::new(Frame {
        pre_context: Annotated::new(vec![Annotated::default(), Annotated::new("".to_string())]),
        post_context: Annotated::new(vec![Annotated::new("".to_string()), Annotated::default()]),
        ..Frame::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut frame, &mut processor, ProcessingState::root()).unwrap();

    let frame = frame.value().unwrap();
    assert_eq_dbg!(frame.context_line.as_str(), Some(""));
}

#[test]
fn test_context_line_retain() {
    let mut frame = Annotated::new(Frame {
        pre_context: Annotated::new(vec![Annotated::default(), Annotated::new("".to_string())]),
        post_context: Annotated::new(vec![Annotated::new("".to_string()), Annotated::default()]),
        context_line: Annotated::new("some line".to_string()),
        ..Frame::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut frame, &mut processor, ProcessingState::root()).unwrap();

    let frame = frame.value().unwrap();
    assert_eq_dbg!(frame.context_line.as_str(), Some("some line"));
}

#[test]
fn test_frame_null_context_lines() {
    let mut frame = Annotated::new(Frame {
        pre_context: Annotated::new(vec![Annotated::default(), Annotated::new("".to_string())]),
        post_context: Annotated::new(vec![Annotated::new("".to_string()), Annotated::default()]),
        ..Frame::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut frame, &mut processor, ProcessingState::root()).unwrap();

    let frame = frame.value().unwrap();
    assert_eq_dbg!(
        *frame.pre_context.value().unwrap(),
        vec![
            Annotated::new("".to_string()),
            Annotated::new("".to_string())
        ],
    );
    assert_eq_dbg!(
        *frame.post_context.value().unwrap(),
        vec![
            Annotated::new("".to_string()),
            Annotated::new("".to_string())
        ],
    );
}

#[test]
fn test_too_long_tags() {
    let mut event = Annotated::new(Event {
        tags: Annotated::new(Tags(PairList(
            vec![Annotated::new(TagEntry(
                Annotated::new("foobar".to_string()),
                Annotated::new("...........................................................................................................................................................................................................".to_string()),
            )), Annotated::new(TagEntry(
                Annotated::new("foooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooo".to_string()),
                Annotated::new("bar".to_string()),
            ))]),
        )),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let event = event.value().unwrap();

    assert_eq_dbg!(
        event.tags.value(),
        Some(&Tags(PairList(vec![
            Annotated::from_error(Error::new(ErrorKind::ValueTooLong), None),
            Annotated::from_error(Error::new(ErrorKind::ValueTooLong), None)
        ])))
    );
}

#[test]
fn test_regression_backfills_abs_path_even_when_moving_stacktrace() {
    use crate::protocol::Values;
    use crate::protocol::{Frame, RawStacktrace};

    let mut event = Annotated::new(Event {
        exceptions: Annotated::new(Values::new(vec![Annotated::new(Exception {
            ty: Annotated::new("FooDivisionError".to_string()),
            value: Annotated::new("hi".to_string().into()),
            ..Exception::default()
        })])),
        stacktrace: Annotated::new(
            RawStacktrace {
                frames: Annotated::new(vec![Annotated::new(Frame {
                    module: Annotated::new("MyModule".to_string()),
                    filename: Annotated::new("MyFilename".into()),
                    function: Annotated::new("Void FooBar()".to_string()),
                    ..Frame::default()
                })]),
                ..RawStacktrace::default()
            }
            .into(),
        ),
        ..Event::default()
    });

    let mut processor = NormalizeProcessor::default();
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    let expected = Annotated::new(
        RawStacktrace {
            frames: Annotated::new(vec![Annotated::new(Frame {
                module: Annotated::new("MyModule".to_string()),
                filename: Annotated::new("MyFilename".into()),
                abs_path: Annotated::new("MyFilename".into()),
                function: Annotated::new("Void FooBar()".to_string()),
                ..Frame::default()
            })]),
            ..RawStacktrace::default()
        }
        .into(),
    );

    assert_eq_dbg!(
        event
            .value()
            .unwrap()
            .exceptions
            .value()
            .unwrap()
            .values
            .value()
            .unwrap()
            .get(0)
            .unwrap()
            .value()
            .unwrap()
            .stacktrace,
        expected
    );
}

#[test]
fn test_parses_sdk_info_from_header() {
    let mut event = Annotated::new(Event::default());
    let mut processor = NormalizeProcessor::new(
        Arc::new(StoreConfig {
            client: Some("_fooBar/0.0.0".to_string()),
            ..StoreConfig::default()
        }),
        None,
    );
    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    assert_eq_dbg!(
        event.value().unwrap().client_sdk,
        Annotated::new(ClientSdkInfo {
            name: Annotated::new("_fooBar".to_string()),
            version: Annotated::new("0.0.0".to_string()),
            ..ClientSdkInfo::default()
        })
    );
}

#[test]
fn test_discards_received() {
    use crate::types::FromValue;
    let mut event = Annotated::new(Event {
        received: FromValue::from_value(Annotated::new(Value::U64(696_969_696_969))),
        ..Default::default()
    });

    let mut processor = NormalizeProcessor::default();

    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    assert!(event.value().unwrap().timestamp.value().is_some());
    assert_eq_dbg!(
        event.value().unwrap().received,
        event.value().unwrap().timestamp,
    );
}

#[test]
fn test_grouping_config() {
    use crate::protocol::LogEntry;
    use crate::types::SerializableAnnotated;
    use insta::assert_ron_snapshot;
    use serde_json::json;

    let mut event = Annotated::new(Event {
        logentry: Annotated::from(LogEntry {
            message: Annotated::new("Hello World!".into()),
            ..Default::default()
        }),
        ..Default::default()
    });

    let mut processor = NormalizeProcessor::new(
        Arc::new(StoreConfig {
            grouping_config: Some(json!({
                "id": "legacy:1234-12-12".to_string(),
            })),
            ..Default::default()
        }),
        None,
    );

    process_value(&mut event, &mut processor, ProcessingState::root()).unwrap();

    assert_ron_snapshot!(SerializableAnnotated(&event), {
        ".event_id" => "[event-id]",
        ".received" => "[received]",
        ".timestamp" => "[timestamp]"
    }, @r###"
    {
      "event_id": "[event-id]",
      "level": "error",
      "type": "default",
      "logentry": {
        "formatted": "Hello World!",
      },
      "logger": "",
      "platform": "other",
      "timestamp": "[timestamp]",
      "received": "[received]",
      "grouping_config": {
        "id": "legacy:1234-12-12",
      },
    }
    "###);
}
