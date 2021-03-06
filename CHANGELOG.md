# Changelog

## 0.5.5

**Store**:

- Small performance improvements in datascrubbing config converter.
- New, C-style selector syntax (old one still works)

**Relay**:

- Suppress verbose DNS lookup logs.

## 0.5.4

**Store**:

- Add event contexts to `pii=maybe`.
- Fix parsing of msgpack breadcrumbs in Rust store.
- Envelopes sent to Rust store can omit the DSN in headers.
- Ability to quote/escape special characters in selectors in PII configs.

## 0.5.3

**Relay**:

- Properly strip the linux binary to reduce its size
- Allow base64 encoded JSON event payloads (#466)
- Fix multipart requests without trailing newline (#465)
- Support for ingesting session updates (#449)

**Store**:

- Validate release names during event ingestion (#479)
- Add browser extension filter (#470)
- Add `pii=maybe`, a new kind of event schema field that can only be scrubbed if explicitly addressed.
- Add way to scrub filepaths in a way that does not break processing.

**Python**:

- Add missing errors for JSON parsing and release validation (#478)
- Expose more datascrubbing utils (#464)

## 0.5.2

**Relay**:

- Fix trivial Redis-related crash when running in non-processing mode.
- Limit the maximum retry-after of a rate limit. This is necessary because of
  the "Delete and ignore future events" feature in Sentry.
- Project caches are now evicted after `project_grace_period` has passed. If
  you have that parameter set to a high number you may see increased memory
  consumption.

**Store**:

- Misc bugfixes in PII processor. Those bugs do not affect the legacy data scrubber exposed in Python.
- Polishing documentation around PII configuration format.
- Signal codes in mach mechanism are no longer required.

## 0.5.1

**Relay**:

- Ability to fetch project configuration from Redis as additional caching layer.
- Fix a few bugs in release filters.
- Fix a few bugs in minidumps endpoint with processing enabled.

**Python**:

- Bump xcode version from 7.3 to 9.4, dropping wheel support for some older OS X versions.
- New function `validate_pii_config`.

**Store**:

- Fix a bug in the PII processor that would always remove the entire string on `pattern` rules.
- Ability to correct some clock drift and wrong system time in transaction events.

## 0.5.0

**Relay**:

- The binary has been renamed to `relay`.
- Updated documentation for metrics.

**Python**:

- The package is now called `sentry-relay`.
- Renamed all `Semaphore*` types to `Relay*`.
- Fixed memory leaks in processing functions.

## 0.4.65

**Relay**:

- Implement the Minidump endpoint.
- Implement the Attachment endpoint.
- Implement the legacy Store endpoint.
- Support a plain `Authorization` header in addition to `X-Sentry-Auth`.
- Simplify the shutdown logic. Relay now always takes a fixed configurable time
  to shut down.
- Fix healthchecks in _Static_ mode.
- Fix internal handling of event attachments.
- Fix partial reads of request bodies resulting in a broken connection.
- Fix a crash when parsing User-Agent headers.
- Fix handling of events sent with `sentry_version=2.0` (Clojure SDK).
- Use _mmap_ to load the GeoIP database to improve the memory footprint.
- Revert back to the system memory allocator.

**Store**:

- Preserve microsecond precision in all time stamps.
- Record event ids in all outcomes.
- Updates to event processing metrics.
- Add span status mapping from open telemetry.

**Python**:

- Fix glob-matching of newline characters.

## 0.4.64

- Added newline support for general glob code.
- Added span status mapping to python library.

**Relay**:

- Switched to jemalloc.
- Introduce separate outcome reason for invalid events.
- Always consume request bodies to the end.
- Implemented minidump ingestion.
- Increas precisions of timestamps in protocol.

## 0.4.63

**Store**:

- Fix a bug where glob-matching in filters did not behave correctly when the
  to-be-matched string contained newlines.
- Add `moz-extension:` as scheme for browser extensions (filtering out Firefox
  addons).
- Raise a dedicated Python exception type for invalid transaction events. Also
  do not report that error to Sentry from Relay.

**Relay**:

- Refactor healthchecks into two: Liveness and readiness (see code comments for
  explanation for now).
- Allow multiple trailing slashes on store endpoint, e.g. `/api/42/store///`.
- Internal refactor to prepare for envelopes format.

## 0.4.62

**Event schema**:

- Spec out values of `event.contexts.trace.status`.
- `none` is now no longer a valid environment name.
- Do no longer drop transaction events in renormalization.

**Store**:

- Various performance improvements.

## 0.4.61

- Add `thread.errored` attribute (#306).

## 0.4.60

- License is now BSL instead of MIT (#301).

**Store**:

- Transaction events with negative duration are now rejected (#291).
- Fix a panic when normalizing certain dates.

**Relay**:

- Improve internal metrics and logging (#296, #297, #298).
- Fix unbounded requests to Sentry for project configs (#295, #300).
- Fix rejected responses from Sentry due to size limit (#303).
- Expose more options for configuring request concurrency limits (#311).

## 0.4.59

- Fix: Normalize legacy stacktrace attributes (#292)
- Fix: Validate platform attributes in Relay (#294)
- Flip the flag that indicates Relay processing (#293)

## 0.4.58

- Expose globbing code from Relay to Python (#288)
- Normalize before datascrubbing (#290)
- Evict project caches after some time (#287)
- Add event size metrics (#286)
- Selectively log internal errors to stderr (#285)
- Do not ignore `process_value` result in `scrub_event` (#284)
- Add a config value for thread counts (#283)
- Refactor outcomes for parity with Sentry (#282)
- Add an error boundary to parsing project states (#281)
- Remove warning and add comment for temporary attribute
- Add flag that relay processed an event (#279)

## 0.4.57

**Store**:

- Stricter validation of transaction events.

## 0.4.56

**Store**:

- Fix a panic in trimming.

## 0.4.55

**Store**:

- Fix more bugs in datascrubbing converter.

## 0.4.54

**Store**:

- Fix more bugs in datascrubbing converter.

## 0.4.53

**Store**:

- Fix more bugs in datascrubbing converter.

## 0.4.52

**Store**:

- Fix more bugs in datascrubbing converter.

## 0.4.51

**Store**:

- Fix a few bugs in datascrubbing converter.
- Accept trailing slashes.

**Normalization**:

- Fix a panic on overflowing timestamps.

## 0.4.50

**Store**:

- Fix bug where IP scrubbers were applied even when not enabled.

## 0.4.49

**Python**

- Fix handling of panics in CABI/Python bindings.

## 0.4.48

**Store**:

- Fix various bugs in the datascrubber and PII processing code to get closer to behavior of the Python implementation.

## 0.4.47

**Normalization**:

- Fix encoding issue in the Python layer of event normalization.

**Store**:

- Various work on re-implementing Sentry's `/api/X/store` endpoint in Relay.
  Relay can now apply rate limits based on Redis and emit the correct outcomes.

## 0.4.46

**Normalization**:

- Resolved a regression in IP address normalization. The new behavior is closer to a line-by-line port of the old Python code.

## 0.4.45

**Normalization**:

- Resolved an issue where GEO IP data was not always infered.

## 0.4.44

**Normalization**:

- Only take the user IP address from the store request's IP for certain platforms. This restores the behavior of the old Python code.

## 0.4.43

**Normalization**:

- Bump size of breadcrumbs.
- Workaround for an issue where we would not parse OS information from User Agent when SDK had already sent OS information.
- Further work on Sentry-internal event ingestion.

## 0.4.42

**Normalization**:

- Fix normalization of version strings from user agents.

## 0.4.41

**Normalization**:

- Parse and normalize user agent strings.

**Relay**:

- Add basic support for Sentry-internal event ingestion.
- Support extended project configuration.
- Implement event filtering rules.

## 0.4.40

- Restrict ranges of timestamps to prevent overflows in Python code and UI.

## 0.4.39

- Fix a bug where stacktrace trimming was not applied during renormalization.

## 0.4.38

- Added typed spans to Event.

## 0.4.37

- Added `orig_in_app` to frame data.

## 0.4.36

- Add new .NET versions for context normalization.

## 0.4.35

- Fix bug where thread's stacktraces were not normalized.
- Fix bug where a string at max depth of a databag was stringified again.

## 0.4.34

- Added `data` attribute to frames.
- Added a way to override other trimming behavior in Python normalizer binding.

## 0.4.33

- Smaller protocol adjustments related to rolling out re-normalization in Rust.
- Plugin-provided context types should now work properly again.

## 0.4.32

- Removed `function_name` field from frame and added `raw_function`.

## 0.4.31

- Add trace context type.

## 0.4.30

- Make exception messages/values larger to allow for foreign stacktrace data to be attached.

## 0.4.29

- Added `function_name` field to frame.

## 0.4.28

- Add missing context type for sessionstack.

## 0.4.27

- Increase frame vars size again! Byte size was fine, but max depth
  was way too small.

## 0.4.26

- Reduce frame vars size.

## 0.4.25

- Add missing trimming to frame vars.

## 0.4.24

- Reject non-http/https `help_urls` in exception mechanisms.

## 0.4.23

- Add basic truncation to event meta to prevent payload size from spiralling out of control.

## 0.4.22

- Added grouping enhancements to protocol.

## 0.4.21

- Updated debug image interface with more attributes.

## 0.4.20

- Added support for `lang` frame and stacktrace attribute.

## 0.4.19

- Slight changes to allow replacing more normalization code in Sentry with Rust.

## 0.4.18

- Allow much larger payloads in the extra attribute.

## 0.4.17

- Added support for protocol changes related to upcoming sentry SDK features.
  In particular the `none` event type was added.

## 0.4.16

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.15

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.14

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.13

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.12

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.11

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.10

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.9

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.8

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.7

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.6

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.5

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.4

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.3

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.2

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.1

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

## 0.4.0

Introducing new Relay modes:

- `proxy`: A proxy for all requests and events.
- `static`: Static configuration for known projects in the file system.
- `managed`: Fetch configurations dynamically from Sentry and update them.

The default Relay mode is `managed`. Users upgrading from previous versions will
automatically activate the `managed` mode. To change this setting, add
`relay.mode` to `config.yml` or run `semaphore config init` from the command
line.

**Breaking Change**: If Relay was used without credentials, the mode needs to be
set to `proxy`. The default `managed` mode requires credentials.

For more information on Relay modes, see the [documentation
page](https://docs.sentry.io/data-management/relay/options/).

### Configuration Changes

- Added `cache.event_buffer_size` to control the maximum number of events that
  are buffered in case of network issues or high rates of incoming events.
- Added `limits.max_concurrent_requests` to limit the number of connections that
  this Relay will use to communicate with the upstream.
- Internal error reporting is now disabled by default. To opt in, set
  `sentry.enabled`.

### Bugfixes

- Fix a bug that caused events to get unconditionally dropped after five
  seconds, regardless of the `cache.event_expiry` configuration.
- Fix a memory leak in Relay's internal error reporting.

## 0.3.0

- Changed PII stripping rule format to permit path selectors when applying
  rules. This means that now `$string` refers to strings for instance and
  `user.id` refers to the `id` field in the `user` attribute of the event.
  Temporarily support for old rules is retained.

## 0.2.7

- store: Minor fixes to be closer to Python. Ability to disable trimming of
  objects, arrays and strings.

## 0.2.6

- Fix bug where PII stripping would remove containers without leaving any
  metadata about the retraction.
- Fix bug where old `redactPair` rules would stop working.

## 0.2.5

- Rewrite of PII stripping logic. This brings potentially breaking changes to
  the semantics of PII configs. Most importantly field types such as
  `"freeform"` and `"databag"` are gone, right now there is only `"container"`
  and `"text"`. All old field types should have become an alias for `"text"`,
  but take extra care in ensuring your PII rules still work.

- store: Minor fixes to be closer to Python.

## 0.2.4

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

- store: Remove stray print statement.

## 0.2.3

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

- store: Fix main performance issues.

## 0.2.2

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

- store: Fix segfault when trying to process contexts.
- store: Fix trimming state "leaking" between interfaces, leading to excessive trimming.
- store: Don't serialize empty arrays and objects (with a few exceptions).

## 0.2.1

For users of relay, nothing changed at all. This is a release to test embedding
some Rust code in Sentry itself.

- `libsemaphore`: Expose CABI for normalizing event data.

## 0.2.0

Our first major iteration on Relay has landed!

- User documentation is now hosted at <https://docs.sentry.io/relay/>.
- SSL support is now included by default. Just configure a [TLS
  identity](https://docs.sentry.io/relay/options/#relaytls_identity_path) and
  you're set.
- Updated event processing: Events from older SDKs are now supported. Also,
  we've fixed some bugs along the line.
- Introduced full support for PII stripping. See [PII
  Configuration](https://docs.sentry.io/relay/pii-config/) for instructions.
- Configure with static project settings. Relay will skip querying project
  states from Sentry and use your provided values instead. See [Project
  Configuration](https://docs.sentry.io/relay/project-config/) for a full guide.
- Relay now also acts as a proxy for certain API requests. This allows it to
  receive CSP reports and Minidump crash reports, among others. It also sets
  `X-Forwarded-For` and includes a Relay signature header.

Besides that, there are many technical changes, including:

- Major rewrite of the internals. Relay no longer requires a special endpoint
  for sending events to upstream Sentry and processes events individually with
  less delay than before.
- The executable will exit with a non-zero exit code on startup errors. This
  makes it easier to catch configuration errors.
- Removed `libsodium` as a production dependency, greatly simplifying
  requirements for the runtime environment.
- Greatly improved logging and metrics. Be careful with the `DEBUG` and `TRACE`
  levels, as they are **very** verbose now.
- Improved docker containers.

## 0.1.3

- Added support for metadata format

## 0.1.2

- JSON logging (#32)
- Update dependencies

## 0.1.1

- Rename "sentry-relay" to "semaphore"
- Use new features from Rust 1.26
- Prepare binary and Python builds (#20)
- Add Dockerfile (#23)

## 0.1.0

An initial release of the tool.
