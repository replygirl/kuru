//! Finite recognizable-secret projection for outward tool results.

use std::{collections::VecDeque, fmt, io};

use serde_json::{Map, Value, map::Entry};

pub(crate) const MARKER: &str = "[REDACTED:recognized-secret]";
const TRUNCATED: &str = "[truncated]";
const GROWTH_FACTOR: usize = 6;
const GROWTH_SLACK: usize = 256;

const CONTEXT_NAMES: &[&[u8]] = &[
    b"openai_api_key",
    b"api_key",
    b"apikey",
    b"aws_access_key_id",
    b"aws_secret_access_key",
    b"aws_session_token",
    b"github_token",
    b"gh_token",
    b"access_token",
    b"refresh_token",
    b"auth_token",
    b"client_secret",
    b"password",
    b"passwd",
    b"private_key",
];
const AUTHORIZATION_NAMES: &[&[u8]] = &[b"Authorization", b"Proxy-Authorization"];

const PRIVATE_DELIMITERS: &[(&[u8], &[u8])] = &[
    (b"-----BEGIN PRIVATE KEY-----", b"-----END PRIVATE KEY-----"),
    (
        b"-----BEGIN ENCRYPTED PRIVATE KEY-----",
        b"-----END ENCRYPTED PRIVATE KEY-----",
    ),
    (
        b"-----BEGIN RSA PRIVATE KEY-----",
        b"-----END RSA PRIVATE KEY-----",
    ),
    (
        b"-----BEGIN EC PRIVATE KEY-----",
        b"-----END EC PRIVATE KEY-----",
    ),
    (
        b"-----BEGIN OPENSSH PRIVATE KEY-----",
        b"-----END OPENSSH PRIVATE KEY-----",
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectionError {
    KeyCollision,
    SizeBound,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::KeyCollision => "tool output was withheld because projected JSON keys collide",
            Self::SizeBound => "tool output was withheld because projection exceeded its bound",
        })
    }
}

impl std::error::Error for ProjectionError {}

#[derive(Clone, Copy)]
enum TokenKind {
    OpenAi,
    GitHub,
}

impl TokenKind {
    fn alphabet(self, byte: u8) -> bool {
        match self {
            Self::OpenAi => byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'),
            Self::GitHub => byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'),
        }
    }

    fn minimum(self) -> usize {
        match self {
            Self::OpenAi => 16,
            Self::GitHub => 8,
        }
    }
}

#[derive(Clone, Copy)]
enum AssignmentKind {
    Context,
    Authorization,
}

#[derive(Clone, Copy)]
enum PrefixKind {
    Token(TokenKind),
    Aws,
    Assignment(AssignmentKind),
    Private(&'static [u8]),
}

#[derive(Clone, Copy)]
struct PrefixMatch {
    length: usize,
    kind: PrefixKind,
}

#[derive(Clone, Copy)]
enum Pattern {
    Plain(&'static [u8]),
    Name {
        name: &'static [u8],
        quote: Option<u8>,
    },
}

impl Pattern {
    fn len(self) -> usize {
        match self {
            Self::Plain(bytes) => bytes.len(),
            Self::Name { name, quote } => name.len() + usize::from(quote.is_some()) * 2,
        }
    }

    fn byte(self, index: usize) -> u8 {
        match self {
            Self::Plain(bytes) => bytes[index],
            Self::Name {
                name,
                quote: Some(quote),
            } => {
                if index == 0 || index == name.len() + 1 {
                    quote
                } else {
                    name[index - 1]
                }
            }
            Self::Name { name, quote: None } => name[index],
        }
    }

    fn starts_with(self, input: &[u8], ascii_fold: bool) -> bool {
        input.len() <= self.len()
            && input.iter().enumerate().all(|(index, input)| {
                let pattern = self.byte(index);
                if ascii_fold {
                    input.eq_ignore_ascii_case(&pattern)
                } else {
                    *input == pattern
                }
            })
    }
}

struct Prefix {
    raw: Vec<u8>,
    name_boundary: bool,
    openai_boundary: bool,
    github_boundary: bool,
    aws_boundary: bool,
    line_start: bool,
}

enum AssignmentPhase {
    AfterName,
    BeforeValue,
    ContextQuotedAwait(u8),
    ContextQuoted {
        quote: u8,
        escaped: bool,
    },
    ContextUnquoted,
    AuthorizationScheme {
        quote: Option<u8>,
        raw: Vec<u8>,
    },
    AuthorizationCredentialAwait {
        quote: Option<u8>,
        scheme: AuthorizationScheme,
    },
    AuthorizationCredential {
        quote: Option<u8>,
        scheme: AuthorizationScheme,
        padding: bool,
    },
}

#[derive(Clone, Copy)]
enum AuthorizationScheme {
    Basic,
    Bearer,
}

enum State {
    Normal,
    Prefix(Prefix),
    TokenPending {
        kind: TokenKind,
        raw: Vec<u8>,
        body: usize,
    },
    TokenSuppress(TokenKind),
    AwsPending {
        raw: Vec<u8>,
        body: usize,
    },
    Assignment {
        kind: AssignmentKind,
        phase: AssignmentPhase,
    },
    PrivateBlock {
        end: &'static [u8],
        matched: usize,
    },
}

pub(crate) trait ProjectionSink {
    fn len(&self) -> usize;

    fn append(
        &mut self,
        bytes: &[u8],
        written: usize,
        buffered: usize,
        limit: usize,
        atomic: bool,
    ) -> Result<Option<usize>, ProjectionError>;
}

impl ProjectionSink for Vec<u8> {
    fn len(&self) -> usize {
        Vec::len(self)
    }

    fn append(
        &mut self,
        bytes: &[u8],
        _written: usize,
        buffered: usize,
        limit: usize,
        _atomic: bool,
    ) -> Result<Option<usize>, ProjectionError> {
        let requested = reserve_for_append(self, buffered, limit)?;
        self.extend_from_slice(bytes);
        Ok(requested)
    }
}

/// Bounded output retained while a scanner continues through the complete input.
pub(crate) struct StreamingProjection {
    scanner: Option<Scanner>,
    output: HeadTail,
}

impl StreamingProjection {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            scanner: Some(Scanner::new()),
            output: HeadTail::new(limit),
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<(), ProjectionError> {
        self.scanner
            .as_mut()
            .expect("streaming projection was already finished")
            .push(bytes, &mut self.output)
    }

    pub(crate) fn finish(mut self) -> Result<String, ProjectionError> {
        self.scanner
            .take()
            .expect("streaming projection was already finished")
            .finish(&mut self.output)?;
        Ok(self.output.into_text())
    }
}

struct HeadTail {
    limit: usize,
    prefix: Vec<u8>,
    tail: VecDeque<u8>,
    prefix_markers: Vec<(usize, usize)>,
    markers: VecDeque<(usize, usize)>,
    written: usize,
    overflowed: bool,
}

impl HeadTail {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            prefix: Vec::with_capacity(limit),
            tail: VecDeque::new(),
            prefix_markers: Vec::new(),
            markers: VecDeque::new(),
            written: 0,
            overflowed: false,
        }
    }

    fn prefix_limit(&self) -> usize {
        self.limit.saturating_sub(TRUNCATED.len()) / 2
    }

    fn tail_limit(&self) -> usize {
        self.limit
            .saturating_sub(TRUNCATED.len())
            .saturating_sub(self.prefix_limit())
    }

    fn trim_prefix(&mut self) {
        let mut end = self.prefix_limit().min(self.prefix.len());
        for &(start, marker_end) in &self.prefix_markers {
            if start < end && end < marker_end {
                end = start;
                break;
            }
        }
        self.prefix.truncate(end);
    }

    fn freeze_prefix(&mut self) {
        if !self.overflowed {
            self.overflowed = true;
            self.trim_prefix();
        }
    }

    fn trim_tail(&mut self) {
        let tail_limit = self.tail_limit();
        let mut start = self.written.saturating_sub(tail_limit);
        while let Some(&(_, end)) = self.markers.front() {
            if end <= start {
                self.markers.pop_front();
            } else {
                break;
            }
        }
        if let Some(&(marker_start, marker_end)) = self.markers.front()
            && marker_start < start
            && start < marker_end
        {
            start = marker_end;
        }
        let current_start = self.written.saturating_sub(self.tail.len());
        for _ in 0..start.saturating_sub(current_start).min(self.tail.len()) {
            self.tail.pop_front();
        }
    }

    fn prefix_without_incomplete_edge(bytes: &[u8]) -> &[u8] {
        match std::str::from_utf8(bytes) {
            Ok(_) => bytes,
            Err(error) if error.error_len().is_none() => &bytes[..error.valid_up_to()],
            Err(_) => bytes,
        }
    }

    fn tail_without_incomplete_edge(bytes: &[u8]) -> &[u8] {
        for start in 0..=bytes.len().min(3) {
            match std::str::from_utf8(&bytes[start..]) {
                Ok(_) => return &bytes[start..],
                Err(error) if error.valid_up_to() > 0 => return &bytes[start..],
                Err(_) => {}
            }
        }
        bytes
    }

    fn append_lossy(output: &mut String, bytes: &[u8]) {
        let mut bytes = bytes;
        while !bytes.is_empty() {
            match std::str::from_utf8(bytes) {
                Ok(valid) => {
                    output.push_str(valid);
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if valid > 0 {
                        output.push_str(
                            std::str::from_utf8(&bytes[..valid]).expect("valid UTF-8 prefix"),
                        );
                    }
                    let invalid = error.error_len().unwrap_or(bytes.len() - valid).max(1);
                    output.push('?');
                    bytes = &bytes[(valid + invalid).min(bytes.len())..];
                }
            }
        }
    }

    fn into_text(self) -> String {
        if !self.overflowed {
            let mut output = String::with_capacity(self.prefix.len());
            Self::append_lossy(&mut output, &self.prefix);
            return output;
        }
        let prefix = Self::prefix_without_incomplete_edge(&self.prefix);
        let tail = self.tail.into_iter().collect::<Vec<_>>();
        let tail = Self::tail_without_incomplete_edge(&tail);
        let mut output = String::with_capacity(prefix.len() + TRUNCATED.len() + tail.len());
        Self::append_lossy(&mut output, prefix);
        output.push_str(TRUNCATED);
        Self::append_lossy(&mut output, tail);
        output
    }
}

impl ProjectionSink for HeadTail {
    fn len(&self) -> usize {
        0
    }

    fn append(
        &mut self,
        bytes: &[u8],
        written: usize,
        _buffered: usize,
        _limit: usize,
        atomic: bool,
    ) -> Result<Option<usize>, ProjectionError> {
        let start = self.written;
        self.written = written;
        if atomic {
            if start < self.limit {
                self.prefix_markers.push((start, written));
            }
            self.markers.push_back((start, written));
        }
        if !self.overflowed {
            if atomic && bytes.len() > self.limit.saturating_sub(self.prefix.len()) {
                self.freeze_prefix();
            } else {
                let keep = bytes
                    .len()
                    .min(self.limit.saturating_sub(self.prefix.len()));
                self.prefix.extend_from_slice(&bytes[..keep]);
                if keep < bytes.len() {
                    self.freeze_prefix();
                }
            }
        }
        let tail_limit = self.tail_limit();
        if tail_limit == 0 {
            self.tail.clear();
        } else if bytes.len() >= tail_limit {
            // `truncate_tool_output` can supply a multi-megabyte ordinary
            // slice. Only its final tail can survive this sink, so never
            // transiently allocate or copy the whole slice.
            self.tail.clear();
            self.tail
                .extend(&bytes[bytes.len().saturating_sub(tail_limit)..]);
        } else {
            self.tail.extend(bytes);
        }
        if self.written > self.limit {
            self.freeze_prefix();
        }
        self.trim_tail();
        Ok(None)
    }
}

/// A bounded-state scanner for byte streams such as a future stderr capture.
pub(crate) struct Scanner {
    state: State,
    previous: Option<u8>,
    line_start: bool,
    input_bytes: usize,
    written_bytes: usize,
    last_was_marker: bool,
    growth_factor: usize,
    growth_slack: usize,
    #[cfg(test)]
    reservation_events: usize,
    #[cfg(test)]
    requested_capacity: usize,
}

impl Scanner {
    pub(crate) fn new() -> Self {
        Self::with_growth_bound(GROWTH_FACTOR, GROWTH_SLACK)
    }

    fn with_growth_bound(growth_factor: usize, growth_slack: usize) -> Self {
        Self {
            state: State::Normal,
            previous: None,
            line_start: true,
            input_bytes: 0,
            written_bytes: 0,
            last_was_marker: false,
            growth_factor,
            growth_slack,
            #[cfg(test)]
            reservation_events: 0,
            #[cfg(test)]
            requested_capacity: 0,
        }
    }

    pub(crate) fn push<S: ProjectionSink>(
        &mut self,
        input: &[u8],
        output: &mut S,
    ) -> Result<(), ProjectionError> {
        self.input_bytes = self
            .input_bytes
            .checked_add(input.len())
            .ok_or(ProjectionError::SizeBound)?;
        self.output_limit()?;
        for &byte in input {
            self.feed(byte, output)?;
        }
        Ok(())
    }

    pub(crate) fn finish<S: ProjectionSink>(
        mut self,
        output: &mut S,
    ) -> Result<(), ProjectionError> {
        loop {
            let state = std::mem::replace(&mut self.state, State::Normal);
            match state {
                State::Normal
                | State::TokenSuppress(_)
                | State::PrivateBlock { .. }
                | State::Assignment {
                    phase:
                        AssignmentPhase::ContextQuoted { .. }
                        | AssignmentPhase::ContextUnquoted
                        | AssignmentPhase::AuthorizationCredential { .. },
                    ..
                } => break,
                State::Prefix(prefix) => {
                    if let Some(found) = best_completed_prefix(&prefix) {
                        let suffix = prefix.raw[found.length..].to_vec();
                        self.start_prefix(found.kind, prefix.raw[..found.length].to_vec(), output)?;
                        for byte in suffix {
                            self.feed(byte, output)?;
                        }
                    } else {
                        self.emit_raw(&prefix.raw, output)?;
                    }
                }
                State::TokenPending { raw, .. } => self.emit_raw(&raw, output)?,
                State::AwsPending { raw, body: 16 } => {
                    self.emit_marker(output)?;
                    self.consume_bytes(&raw);
                }
                State::AwsPending { raw, .. } => self.emit_raw(&raw, output)?,
                State::Assignment {
                    kind: _,
                    phase: AssignmentPhase::AuthorizationScheme { raw, .. },
                } => {
                    for byte in raw {
                        self.feed(byte, output)?;
                    }
                }
                State::Assignment { .. } => break,
            }
        }
        self.output_limit().map(|_| ())
    }

    fn feed<S: ProjectionSink>(&mut self, byte: u8, output: &mut S) -> Result<(), ProjectionError> {
        let state = std::mem::replace(&mut self.state, State::Normal);
        match state {
            State::Normal => self.feed_normal(byte, output),
            State::Prefix(mut prefix) => {
                prefix.raw.push(byte);
                if has_strict_prefix(&prefix) {
                    self.state = State::Prefix(prefix);
                    return Ok(());
                }
                if let Some(found) = best_completed_prefix(&prefix) {
                    let suffix = prefix.raw[found.length..].to_vec();
                    self.start_prefix(found.kind, prefix.raw[..found.length].to_vec(), output)?;
                    for byte in suffix {
                        self.feed(byte, output)?;
                    }
                    return Ok(());
                }
                let mut raw = prefix.raw.into_iter();
                if let Some(first) = raw.next() {
                    self.emit_byte(first, output)?;
                }
                for byte in raw {
                    self.feed(byte, output)?;
                }
                Ok(())
            }
            State::TokenPending {
                kind,
                mut raw,
                mut body,
            } => {
                if kind.alphabet(byte) {
                    raw.push(byte);
                    body += 1;
                    if body == kind.minimum() {
                        self.emit_marker(output)?;
                        self.consume_bytes(&raw);
                        self.state = State::TokenSuppress(kind);
                    } else {
                        self.state = State::TokenPending { kind, raw, body };
                    }
                    Ok(())
                } else {
                    self.emit_raw(&raw, output)?;
                    self.feed(byte, output)
                }
            }
            State::TokenSuppress(kind) => {
                if kind.alphabet(byte) {
                    self.consume_byte(byte);
                    self.state = State::TokenSuppress(kind);
                    Ok(())
                } else {
                    self.feed(byte, output)
                }
            }
            State::AwsPending { mut raw, mut body } => {
                if body < 16 && (byte.is_ascii_uppercase() || byte.is_ascii_digit()) {
                    raw.push(byte);
                    body += 1;
                    self.state = State::AwsPending { raw, body };
                    Ok(())
                } else if body == 16 && !byte.is_ascii_uppercase() && !byte.is_ascii_digit() {
                    self.emit_marker(output)?;
                    self.consume_bytes(&raw);
                    self.feed(byte, output)
                } else {
                    raw.push(byte);
                    self.emit_raw(&raw, output)
                }
            }
            State::Assignment { kind, phase } => self.feed_assignment(kind, phase, byte, output),
            State::PrivateBlock { end, matched } => {
                self.consume_byte(byte);
                let matched = advance_fixed_match(end, matched, byte);
                if matched == end.len() {
                    self.state = State::Normal;
                } else {
                    self.state = State::PrivateBlock { end, matched };
                }
                Ok(())
            }
        }
    }

    fn feed_normal<S: ProjectionSink>(
        &mut self,
        byte: u8,
        output: &mut S,
    ) -> Result<(), ProjectionError> {
        let prefix = Prefix {
            raw: vec![byte],
            name_boundary: self.previous.is_none_or(|previous| !is_name_byte(previous)),
            openai_boundary: self
                .previous
                .is_none_or(|previous| !TokenKind::OpenAi.alphabet(previous)),
            github_boundary: self
                .previous
                .is_none_or(|previous| !TokenKind::GitHub.alphabet(previous)),
            aws_boundary: self.previous.is_none_or(|previous| {
                !previous.is_ascii_uppercase() && !previous.is_ascii_digit()
            }),
            line_start: self.line_start,
        };
        if has_any_prefix(&prefix) {
            self.state = State::Prefix(prefix);
            Ok(())
        } else {
            self.emit_byte(byte, output)
        }
    }

    fn start_prefix<S: ProjectionSink>(
        &mut self,
        kind: PrefixKind,
        raw: Vec<u8>,
        output: &mut S,
    ) -> Result<(), ProjectionError> {
        match kind {
            PrefixKind::Token(kind) => {
                self.state = State::TokenPending { kind, raw, body: 0 };
            }
            PrefixKind::Aws => {
                self.state = State::AwsPending { raw, body: 0 };
            }
            PrefixKind::Assignment(kind) => {
                self.emit_raw(&raw, output)?;
                self.state = State::Assignment {
                    kind,
                    phase: AssignmentPhase::AfterName,
                };
            }
            PrefixKind::Private(end) => {
                self.emit_marker(output)?;
                self.consume_bytes(&raw);
                self.state = State::PrivateBlock { end, matched: 0 };
            }
        }
        Ok(())
    }

    fn feed_assignment<S: ProjectionSink>(
        &mut self,
        kind: AssignmentKind,
        phase: AssignmentPhase,
        byte: u8,
        output: &mut S,
    ) -> Result<(), ProjectionError> {
        match phase {
            AssignmentPhase::AfterName => {
                if is_horizontal_space(byte) {
                    self.emit_byte(byte, output)?;
                    self.assignment(kind, AssignmentPhase::AfterName);
                } else if matches!(kind, AssignmentKind::Context) && matches!(byte, b'=' | b':')
                    || matches!(kind, AssignmentKind::Authorization) && byte == b':'
                {
                    self.emit_byte(byte, output)?;
                    self.assignment(kind, AssignmentPhase::BeforeValue);
                } else {
                    self.feed(byte, output)?;
                }
            }
            AssignmentPhase::BeforeValue => {
                if is_horizontal_space(byte) {
                    self.emit_byte(byte, output)?;
                    self.assignment(kind, AssignmentPhase::BeforeValue);
                } else {
                    match kind {
                        AssignmentKind::Context if matches!(byte, b'\'' | b'"') => {
                            self.emit_byte(byte, output)?;
                            self.assignment(kind, AssignmentPhase::ContextQuotedAwait(byte));
                        }
                        AssignmentKind::Context if is_unquoted_delimiter(byte) => {
                            self.feed(byte, output)?;
                        }
                        AssignmentKind::Context => {
                            self.emit_marker(output)?;
                            self.consume_byte(byte);
                            self.assignment(kind, AssignmentPhase::ContextUnquoted);
                        }
                        AssignmentKind::Authorization if matches!(byte, b'\'' | b'"') => {
                            self.emit_byte(byte, output)?;
                            self.assignment(
                                kind,
                                AssignmentPhase::AuthorizationScheme {
                                    quote: Some(byte),
                                    raw: Vec::new(),
                                },
                            );
                        }
                        AssignmentKind::Authorization => {
                            self.assignment(
                                kind,
                                AssignmentPhase::AuthorizationScheme {
                                    quote: None,
                                    raw: Vec::new(),
                                },
                            );
                            self.feed(byte, output)?;
                        }
                    }
                }
            }
            AssignmentPhase::ContextQuotedAwait(quote) => {
                if byte == quote {
                    self.emit_byte(byte, output)?;
                    self.state = State::Normal;
                } else {
                    self.emit_marker(output)?;
                    self.consume_byte(byte);
                    self.assignment(
                        kind,
                        AssignmentPhase::ContextQuoted {
                            quote,
                            escaped: byte == b'\\',
                        },
                    );
                }
            }
            AssignmentPhase::ContextQuoted { quote, escaped } => {
                if byte == quote && !escaped {
                    self.emit_byte(byte, output)?;
                    self.state = State::Normal;
                } else {
                    self.consume_byte(byte);
                    self.assignment(
                        kind,
                        AssignmentPhase::ContextQuoted {
                            quote,
                            escaped: !escaped && byte == b'\\',
                        },
                    );
                }
            }
            AssignmentPhase::ContextUnquoted => {
                if is_unquoted_delimiter(byte) {
                    self.feed(byte, output)?;
                } else {
                    self.consume_byte(byte);
                    self.assignment(kind, AssignmentPhase::ContextUnquoted);
                }
            }
            AssignmentPhase::AuthorizationScheme { quote, mut raw } => {
                raw.push(byte);
                if let Some(scheme) = complete_authorization_scheme(&raw) {
                    self.emit_raw(&raw, output)?;
                    self.assignment(
                        kind,
                        AssignmentPhase::AuthorizationCredentialAwait { quote, scheme },
                    );
                } else if authorization_scheme_prefix(&raw) {
                    self.assignment(kind, AssignmentPhase::AuthorizationScheme { quote, raw });
                } else {
                    for byte in raw {
                        self.feed(byte, output)?;
                    }
                }
            }
            AssignmentPhase::AuthorizationCredentialAwait { quote, scheme } => {
                if is_horizontal_space(byte) {
                    self.emit_byte(byte, output)?;
                    self.assignment(
                        kind,
                        AssignmentPhase::AuthorizationCredentialAwait { quote, scheme },
                    );
                } else if quote == Some(byte) || is_line_end(byte) {
                    self.feed(byte, output)?;
                } else if authorization_body_byte(scheme, byte) {
                    self.emit_marker(output)?;
                    self.consume_byte(byte);
                    self.assignment(
                        kind,
                        AssignmentPhase::AuthorizationCredential {
                            quote,
                            scheme,
                            padding: false,
                        },
                    );
                } else {
                    self.feed(byte, output)?;
                }
            }
            AssignmentPhase::AuthorizationCredential {
                quote,
                scheme,
                mut padding,
            } => {
                let accepted = match scheme {
                    AuthorizationScheme::Basic => is_basic_byte(byte),
                    AuthorizationScheme::Bearer if byte == b'=' => {
                        padding = true;
                        true
                    }
                    AuthorizationScheme::Bearer => !padding && is_bearer_byte(byte),
                };
                if accepted {
                    self.consume_byte(byte);
                    self.assignment(
                        kind,
                        AssignmentPhase::AuthorizationCredential {
                            quote,
                            scheme,
                            padding,
                        },
                    );
                } else {
                    self.feed(byte, output)?;
                }
            }
        }
        Ok(())
    }

    fn assignment(&mut self, kind: AssignmentKind, phase: AssignmentPhase) {
        self.state = State::Assignment { kind, phase };
    }

    fn emit_marker<S: ProjectionSink>(&mut self, output: &mut S) -> Result<(), ProjectionError> {
        if !self.last_was_marker {
            self.append(MARKER.as_bytes(), output, true)?;
            self.last_was_marker = true;
        }
        Ok(())
    }

    fn emit_byte<S: ProjectionSink>(
        &mut self,
        byte: u8,
        output: &mut S,
    ) -> Result<(), ProjectionError> {
        self.append(&[byte], output, false)?;
        self.consume_byte(byte);
        self.last_was_marker = false;
        Ok(())
    }

    fn emit_raw<S: ProjectionSink>(
        &mut self,
        raw: &[u8],
        output: &mut S,
    ) -> Result<(), ProjectionError> {
        self.append(raw, output, false)?;
        self.consume_bytes(raw);
        if !raw.is_empty() {
            self.last_was_marker = false;
        }
        Ok(())
    }

    fn append<S: ProjectionSink>(
        &mut self,
        bytes: &[u8],
        output: &mut S,
        atomic: bool,
    ) -> Result<(), ProjectionError> {
        let written = self
            .written_bytes
            .checked_add(bytes.len())
            .ok_or(ProjectionError::SizeBound)?;
        let output_limit = self.output_limit()?;
        ensure_within(written, output_limit)?;
        let buffered = output
            .len()
            .checked_add(bytes.len())
            .ok_or(ProjectionError::SizeBound)?;
        if let Some(_requested_capacity) =
            output.append(bytes, written, buffered, output_limit, atomic)?
        {
            #[cfg(test)]
            {
                self.reservation_events += 1;
                self.requested_capacity = _requested_capacity;
            }
        }
        self.written_bytes = written;
        Ok(())
    }

    fn output_limit(&self) -> Result<usize, ProjectionError> {
        self.input_bytes
            .checked_mul(self.growth_factor)
            .and_then(|bound| bound.checked_add(self.growth_slack))
            .ok_or(ProjectionError::SizeBound)
    }

    fn consume_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.consume_byte(byte);
        }
    }

    fn consume_byte(&mut self, byte: u8) {
        self.previous = Some(byte);
        if is_line_end(byte) {
            self.line_start = true;
        } else if self.line_start && is_horizontal_space(byte) {
            // Leading indentation retains line-start eligibility.
        } else {
            self.line_start = false;
        }
    }

    #[cfg(test)]
    fn pending_bytes(&self) -> usize {
        match &self.state {
            State::Prefix(prefix) => prefix.raw.len(),
            State::TokenPending { raw, .. }
            | State::AwsPending { raw, .. }
            | State::Assignment {
                phase: AssignmentPhase::AuthorizationScheme { raw, .. },
                ..
            } => raw.len(),
            _ => 0,
        }
    }

    #[cfg(test)]
    fn reservation_events(&self) -> usize {
        self.reservation_events
    }

    #[cfg(test)]
    fn requested_capacity(&self) -> usize {
        self.requested_capacity
    }
}

pub(crate) fn text(input: &str) -> Result<String, ProjectionError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(input.len())
        .map_err(|_| ProjectionError::SizeBound)?;
    let mut scanner = Scanner::new();
    scanner.push(input.as_bytes(), &mut output)?;
    scanner.finish(&mut output)?;
    String::from_utf8(output).map_err(|_| ProjectionError::SizeBound)
}

pub(crate) fn json(input: Value) -> Result<String, ProjectionError> {
    let mut original = CountingWriter::default();
    serde_json::to_writer(&mut original, &input).map_err(|_| ProjectionError::SizeBound)?;
    let limit = relative_bound(original.bytes)?;
    let projected = project_value(input)?;
    let mut output = BoundedWriter::new(limit);
    serde_json::to_writer(&mut output, &projected).map_err(|_| ProjectionError::SizeBound)?;
    String::from_utf8(output.bytes).map_err(|_| ProjectionError::SizeBound)
}

#[derive(Default)]
struct CountingWriter {
    bytes: usize,
}

impl io::Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("serialized tool output length overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
    #[cfg(test)]
    reservation_events: usize,
    #[cfg(test)]
    requested_capacity: usize,
}

impl BoundedWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            #[cfg(test)]
            reservation_events: 0,
            #[cfg(test)]
            requested_capacity: 0,
        }
    }
}

impl io::Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|written| *written <= self.limit)
            .ok_or_else(|| io::Error::other("projected tool output exceeds its bound"))?;
        if let Some(_requested_capacity) = reserve_for_append(&mut self.bytes, written, self.limit)
            .map_err(|_| io::Error::other("projected tool output allocation failed"))?
        {
            #[cfg(test)]
            {
                self.reservation_events += 1;
                self.requested_capacity = _requested_capacity;
            }
        }
        self.bytes.extend_from_slice(bytes);
        debug_assert_eq!(self.bytes.len(), written);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn reserve_for_append(
    output: &mut Vec<u8>,
    required: usize,
    limit: usize,
) -> Result<Option<usize>, ProjectionError> {
    ensure_within(required, limit)?;
    if output.capacity() >= required {
        return Ok(None);
    }

    let doubled = output.capacity().checked_mul(2).unwrap_or(limit);
    let target = doubled.max(required).min(limit);
    output
        .try_reserve_exact(target - output.len())
        .map_err(|_| ProjectionError::SizeBound)?;
    Ok(Some(target))
}

fn project_value(input: Value) -> Result<Value, ProjectionError> {
    match input {
        Value::String(value) => Ok(Value::String(text(&value)?)),
        Value::Array(values) => values
            .into_iter()
            .map(project_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) => {
            let mut output = Map::new();
            for (key, value) in values {
                let selected = sensitive_json_key(&key);
                let key = text(&key)?;
                let value = if selected {
                    Value::String(MARKER.into())
                } else {
                    project_value(value)?
                };
                match output.entry(key) {
                    Entry::Vacant(entry) => {
                        entry.insert(value);
                    }
                    Entry::Occupied(_) => return Err(ProjectionError::KeyCollision),
                }
            }
            Ok(Value::Object(output))
        }
        scalar => Ok(scalar),
    }
}

fn relative_bound(input: usize) -> Result<usize, ProjectionError> {
    input
        .checked_mul(GROWTH_FACTOR)
        .and_then(|bound| bound.checked_add(GROWTH_SLACK))
        .ok_or(ProjectionError::SizeBound)
}

fn ensure_within(actual: usize, bound: usize) -> Result<(), ProjectionError> {
    if actual <= bound {
        Ok(())
    } else {
        Err(ProjectionError::SizeBound)
    }
}

fn sensitive_json_key(key: &str) -> bool {
    CONTEXT_NAMES
        .iter()
        .chain(AUTHORIZATION_NAMES)
        .any(|name| key.as_bytes().eq_ignore_ascii_case(name))
}

fn has_any_prefix(prefix: &Prefix) -> bool {
    has_strict_prefix(prefix) || best_completed_prefix(prefix).is_some()
}

fn has_strict_prefix(prefix: &Prefix) -> bool {
    for_each_prefix(prefix, |pattern, _, ascii_fold| {
        pattern.len() > prefix.raw.len() && pattern.starts_with(&prefix.raw, ascii_fold)
    })
}

fn best_completed_prefix(prefix: &Prefix) -> Option<PrefixMatch> {
    let mut best = None;
    for_each_prefix(prefix, |pattern, kind, ascii_fold| {
        if pattern.len() <= prefix.raw.len()
            && pattern.starts_with(&prefix.raw[..pattern.len()], ascii_fold)
            && best.is_none_or(|current: PrefixMatch| pattern.len() > current.length)
        {
            best = Some(PrefixMatch {
                length: pattern.len(),
                kind,
            });
        }
        false
    });
    best
}

fn for_each_prefix(
    prefix: &Prefix,
    mut visit: impl FnMut(Pattern, PrefixKind, bool) -> bool,
) -> bool {
    if prefix.openai_boundary {
        for pattern in [b"sk-svcacct-".as_slice(), b"sk-proj-", b"sk-"] {
            if visit(
                Pattern::Plain(pattern),
                PrefixKind::Token(TokenKind::OpenAi),
                false,
            ) {
                return true;
            }
        }
    }
    if prefix.github_boundary {
        for pattern in [
            b"github_pat_".as_slice(),
            b"ghp_",
            b"gho_",
            b"ghu_",
            b"ghs_",
            b"ghr_",
        ] {
            if visit(
                Pattern::Plain(pattern),
                PrefixKind::Token(TokenKind::GitHub),
                false,
            ) {
                return true;
            }
        }
    }
    if prefix.aws_boundary {
        for pattern in [b"AKIA".as_slice(), b"ASIA"] {
            if visit(Pattern::Plain(pattern), PrefixKind::Aws, false) {
                return true;
            }
        }
    }
    if prefix.name_boundary {
        for &name in CONTEXT_NAMES {
            for quote in [None, Some(b'\''), Some(b'"')] {
                if visit(
                    Pattern::Name { name, quote },
                    PrefixKind::Assignment(AssignmentKind::Context),
                    true,
                ) {
                    return true;
                }
            }
        }
        for &name in AUTHORIZATION_NAMES {
            for quote in [None, Some(b'\''), Some(b'"')] {
                if visit(
                    Pattern::Name { name, quote },
                    PrefixKind::Assignment(AssignmentKind::Authorization),
                    true,
                ) {
                    return true;
                }
            }
        }
    }
    if prefix.line_start {
        for &(begin, end) in PRIVATE_DELIMITERS {
            if visit(Pattern::Plain(begin), PrefixKind::Private(end), false) {
                return true;
            }
        }
    }
    false
}

fn complete_authorization_scheme(raw: &[u8]) -> Option<AuthorizationScheme> {
    for (name, scheme) in [
        (b"Basic".as_slice(), AuthorizationScheme::Basic),
        (b"Bearer".as_slice(), AuthorizationScheme::Bearer),
    ] {
        if raw.len() == name.len() + 1
            && raw[..name.len()].eq_ignore_ascii_case(name)
            && is_horizontal_space(raw[name.len()])
        {
            return Some(scheme);
        }
    }
    None
}

fn authorization_scheme_prefix(raw: &[u8]) -> bool {
    [b"Basic".as_slice(), b"Bearer"]
        .iter()
        .any(|name| raw.len() <= name.len() && name[..raw.len()].eq_ignore_ascii_case(raw))
}

fn authorization_body_byte(scheme: AuthorizationScheme, byte: u8) -> bool {
    match scheme {
        AuthorizationScheme::Basic => is_basic_byte(byte),
        AuthorizationScheme::Bearer => is_bearer_byte(byte),
    }
}

fn is_basic_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
}

fn is_bearer_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_horizontal_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t')
}

fn is_line_end(byte: u8) -> bool {
    matches!(byte, b'\r' | b'\n')
}

fn is_unquoted_delimiter(byte: u8) -> bool {
    is_horizontal_space(byte) || is_line_end(byte) || matches!(byte, b',' | b';' | b'}' | b']')
}

fn advance_fixed_match(pattern: &[u8], matched: usize, byte: u8) -> usize {
    let total = matched + 1;
    for length in (0..=pattern.len().min(total)).rev() {
        let start = total - length;
        let matches = (0..length).all(|index| {
            let source = if start + index < matched {
                pattern[start + index]
            } else {
                byte
            };
            source == pattern[index]
        });
        if matches {
            return length;
        }
    }
    0
}

/// Truncate already-projected tool output without emitting a partial marker.
pub fn truncate_tool_output(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    if max_bytes < TRUNCATED.len() {
        return ".".repeat(max_bytes.min(3));
    }
    let mut output = HeadTail::new(max_bytes);
    let mut written = 0;
    let mut start = 0;
    for (index, marker) in text.match_indices(MARKER) {
        let prefix = &text.as_bytes()[start..index];
        written += prefix.len();
        output
            .append(prefix, written, 0, usize::MAX, false)
            .expect("fixed head/tail sink accepts bounded text");
        written += marker.len();
        output
            .append(marker.as_bytes(), written, 0, usize::MAX, true)
            .expect("fixed head/tail sink accepts bounded text");
        start = index + marker.len();
    }
    let suffix = &text.as_bytes()[start..];
    written += suffix.len();
    output
        .append(suffix, written, 0, usize::MAX, false)
        .expect("fixed head/tail sink accepts bounded text");
    output.into_text()
}

#[cfg(test)]
mod tests {
    use std::io;

    use serde_json::{Map, Value, json};

    use super::{
        BoundedWriter, CountingWriter, MARKER, ProjectionError, Scanner, TRUNCATED, json,
        project_value, relative_bound, text, truncate_tool_output,
    };

    fn streamed(input: &[u8], splits: &[usize]) -> Result<Vec<u8>, ProjectionError> {
        let mut scanner = Scanner::new();
        let mut output = Vec::new();
        let mut start = 0;
        for &end in splits {
            scanner.push(&input[start..end], &mut output)?;
            start = end;
        }
        scanner.push(&input[start..], &mut output)?;
        scanner.finish(&mut output)?;
        Ok(output)
    }

    #[test]
    fn finite_detectors_preserve_syntax_and_replace_only_recognized_spans() {
        let cases = [
            (
                "Authorization: Bearer abc.DEF-~_+/== tail",
                format!("Authorization: Bearer {MARKER} tail"),
            ),
            (
                "\"Proxy-Authorization\" : 'bAsIc QWxhZGRpbjpvcGVuIHNlc2FtZQ=='",
                format!("\"Proxy-Authorization\" : 'bAsIc {MARKER}'"),
            ),
            ("api_key=x; keep", format!("api_key={MARKER}; keep")),
            (
                "\"password\": 1234, next",
                format!("\"password\": {MARKER}, next"),
            ),
            (
                "'client_secret' = 'a\\'b'",
                format!("'client_secret' = '{MARKER}'"),
            ),
            (
                "password=\"first\nsecond\" after",
                format!("password=\"{MARKER}\" after"),
            ),
            (
                "private_key=\"\nBEGIN\r\nbody\n\" after",
                format!("private_key=\"{MARKER}\" after"),
            ),
            ("sk-abcdefghijklmnop", MARKER.into()),
            ("sk-proj-abcdefghijklmnop", MARKER.into()),
            ("sk-svcacct-abcdefghijklmnop", MARKER.into()),
            ("ghp_abcdefgh", MARKER.into()),
            ("github_pat_abcdefgh", MARKER.into()),
            ("AKIA1234567890ABCDEF", MARKER.into()),
            (
                "  -----BEGIN PRIVATE KEY-----\nbody\n-----END PRIVATE KEY-----\nafter",
                format!("  {MARKER}\nafter"),
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(text(input).unwrap(), expected, "input: {input}");
        }

        for prefix in ["ghp_", "github_pat_", "gho_", "ghu_", "ghs_", "ghr_"] {
            let input = format!("{prefix}abcdefgh");
            assert_eq!(text(&input).unwrap(), MARKER, "prefix: {prefix}");
        }
        for prefix in ["AKIA", "ASIA"] {
            let input = format!("{prefix}1234567890ABCDEF");
            assert_eq!(text(&input).unwrap(), MARKER, "prefix: {prefix}");
        }
        for (begin, end) in super::PRIVATE_DELIMITERS {
            let input = format!(
                "{}\nbody\n{}\nkept",
                String::from_utf8_lossy(begin),
                String::from_utf8_lossy(end)
            );
            assert_eq!(text(&input).unwrap(), format!("{MARKER}\nkept"));
        }
        for name in super::CONTEXT_NAMES {
            let name = String::from_utf8_lossy(name);
            assert_eq!(
                text(&format!("{name}:opaque")).unwrap(),
                format!("{name}:{MARKER}"),
                "name: {name}"
            );
            assert_eq!(
                text(&format!("\"{}\" = 'opaque'", name.to_ascii_uppercase())).unwrap(),
                format!("\"{}\" = '{MARKER}'", name.to_ascii_uppercase()),
                "quoted name: {name}"
            );
        }
    }

    #[test]
    fn every_byte_split_and_one_byte_chunks_equal_whole_projection() {
        let cases = [
            "before Authorization: Bearer abc.def== after",
            "before \"api_key\"  :  \"opaque\\\"tail\" after",
            "before password=\"first\nsecond\" after",
            "before private_key=\"\nBEGIN\r\nbody\n\" after",
            "before sk-proj-abcdefghijklmnop after",
            "before github_pat_abcdefgh after",
            "before AKIA1234567890ABCDEF after",
            "  -----BEGIN OPENSSH PRIVATE KEY-----\nbody\n-----END OPENSSH PRIVATE KEY-----\nafter",
        ];
        for input in cases {
            let expected = text(input).unwrap().into_bytes();
            for split in 0..=input.len() {
                assert_eq!(
                    streamed(input.as_bytes(), &[split]).unwrap(),
                    expected,
                    "input: {input}, split: {split}"
                );
            }
            assert_eq!(
                streamed(input.as_bytes(), &(0..=input.len()).collect::<Vec<_>>()).unwrap(),
                expected,
                "one-byte chunks: {input}"
            );
        }
    }

    #[test]
    fn boundaries_floors_and_unrecognized_controls_remain_exact() {
        let controls = [
            "passwordless=x",
            "xapi_key=y",
            "Authorization: Digest opaque",
            "sk-abcdefghijklmno",
            "ghp_abcdefg",
            "AKIA1234567890ABCDEFG",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "550e8400-e29b-41d4-a716-446655440000",
            "gpt-6.0-model",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.signature",
            "-----BEGIN CERTIFICATE-----\nopaque\n-----END CERTIFICATE-----",
            "-----BEGIN PUBLIC KEY-----\nopaque\n-----END PUBLIC KEY-----",
            "prefix sk_new_format_abcdefghijklmnop",
        ];
        for input in controls {
            assert_eq!(text(input).unwrap(), input);
        }
        assert_eq!(
            text("x.sk-abcdefghijklmnop").unwrap(),
            format!("x.{MARKER}")
        );
        assert_eq!(
            text("XAKIA1234567890ABCDEF").unwrap(),
            "XAKIA1234567890ABCDEF"
        );
        assert_eq!(text("AKIA1234567890ABCDEFx").unwrap(), format!("{MARKER}x"));
    }

    #[test]
    fn eof_flushes_only_unrecognized_candidates() {
        assert_eq!(text("sk-abcdefghijklmno").unwrap(), "sk-abcdefghijklmno");
        assert_eq!(text("AKIA1234567890ABCDEF").unwrap(), MARKER);
        assert_eq!(text("api_key='").unwrap(), "api_key='");
        assert_eq!(
            text("api_key='secret").unwrap(),
            format!("api_key='{MARKER}")
        );
        assert_eq!(
            text("-----BEGIN RSA PRIVATE KEY-----\nunfinished").unwrap(),
            MARKER
        );
    }

    #[test]
    fn scanner_state_stays_fixed_across_long_runs() {
        let mut scanner = Scanner::new();
        let mut output = Vec::new();
        scanner.push(b"api_key", &mut output).unwrap();
        scanner.push(&vec![b' '; 32_768], &mut output).unwrap();
        assert_eq!(scanner.pending_bytes(), 0);
        scanner.push(b"=value", &mut output).unwrap();
        assert_eq!(scanner.pending_bytes(), 0);
        scanner.finish(&mut output).unwrap();
        assert!(output.ends_with(MARKER.as_bytes()));

        let mut scanner = Scanner::new();
        let mut private = Vec::new();
        scanner
            .push(b"-----BEGIN PRIVATE KEY-----", &mut private)
            .unwrap();
        scanner.push(&vec![b'x'; 32_768], &mut private).unwrap();
        assert_eq!(scanner.pending_bytes(), 0);
        scanner.finish(&mut private).unwrap();
        assert_eq!(private, MARKER.as_bytes());

        let mut scanner = Scanner::new();
        let mut quoted = Vec::new();
        scanner.push(b"password='", &mut quoted).unwrap();
        scanner.push(&vec![b'x'; 32_768], &mut quoted).unwrap();
        assert_eq!(scanner.pending_bytes(), 0);
        scanner.finish(&mut quoted).unwrap();
        assert_eq!(quoted, format!("password='{MARKER}").as_bytes());
    }

    #[test]
    fn typed_json_selects_sensitive_values_and_preserves_unknown_structure() {
        let input = json!({
            "api_key": "opaque",
            "PASSWORD": 1234,
            "Authorization": ["Bearer opaque", {"nested": true}],
            "client_secret": false,
            "private_key": null,
            "refresh_token": {"unknown": [1, 2, 3]},
            "nested": {
                "safe": "prefix sk-abcdefghijklmnop suffix",
                "boolean": true,
                "null": null,
                "array": ["ghp_abcdefgh", 7]
            }
        });
        let projected: Value = serde_json::from_str(&json(input).unwrap()).unwrap();
        assert_eq!(projected["api_key"], MARKER);
        assert_eq!(projected["PASSWORD"], MARKER);
        assert_eq!(projected["Authorization"], MARKER);
        assert_eq!(projected["client_secret"], MARKER);
        assert_eq!(projected["private_key"], MARKER);
        assert_eq!(projected["refresh_token"], MARKER);
        assert_eq!(
            projected["nested"]["safe"],
            format!("prefix {MARKER} suffix")
        );
        assert_eq!(projected["nested"]["boolean"], true);
        assert!(projected["nested"]["null"].is_null());
        assert_eq!(projected["nested"]["array"], json!([MARKER, 7]));
    }

    #[test]
    fn json_projects_key_spans_and_refuses_collisions() {
        let projected: Value = serde_json::from_str(
            &json(json!({"prefix sk-abcdefghijklmnop suffix": "kept"})).unwrap(),
        )
        .unwrap();
        assert_eq!(projected[format!("prefix {MARKER} suffix")], "kept");

        let collision = json(json!({
            "prefix sk-abcdefghijklmnop": 1,
            "prefix sk-qrstuvwxyzabcdef": 2
        }));
        assert_eq!(collision.unwrap_err(), ProjectionError::KeyCollision);
    }

    #[test]
    fn arbitrary_json_looking_text_is_not_parsed_or_reformatted() {
        let input = " { \"api_key\" : \"opaque\", \"safe\": 1 } ";
        assert_eq!(
            text(input).unwrap(),
            format!(" {{ \"api_key\" : \"{MARKER}\", \"safe\": 1 }} ")
        );
        let header = "prefix \"Authorization\": \"Bearer opaque\" suffix";
        assert_eq!(
            text(header).unwrap(),
            format!("prefix \"Authorization\": \"Bearer {MARKER}\" suffix")
        );
    }

    #[test]
    fn impossible_growth_bound_fails_with_fixed_error() {
        let mut scanner = Scanner::with_growth_bound(0, 0);
        let mut output = Vec::new();
        assert_eq!(
            scanner.push(b"api_key=x", &mut output).unwrap_err(),
            ProjectionError::SizeBound
        );
        assert_eq!(
            ProjectionError::SizeBound.to_string(),
            "tool output was withheld because projection exceeded its bound"
        );

        assert_eq!(relative_bound(usize::MAX), Err(ProjectionError::SizeBound));

        let mut scanner = Scanner::new();
        scanner.input_bytes = usize::MAX;
        assert_eq!(
            scanner.push(b"x", &mut Vec::new()),
            Err(ProjectionError::SizeBound)
        );

        let mut scanner = Scanner::new();
        scanner.written_bytes = usize::MAX;
        assert_eq!(
            scanner.push(b"x", &mut Vec::new()),
            Err(ProjectionError::SizeBound)
        );

        let mut original = CountingWriter { bytes: usize::MAX };
        assert!(io::Write::write(&mut original, b"x").is_err());

        let mut writer = BoundedWriter::new(1);
        assert!(io::Write::write(&mut writer, b"xx").is_err());
        assert!(writer.bytes.is_empty());
        assert_eq!(writer.reservation_events, 0);
    }

    #[test]
    fn projected_buffers_request_bounded_geometric_growth() {
        let input = "passwd=x;".repeat(4_096);
        let mut scanner = Scanner::new();
        let mut output = Vec::new();
        for byte in input.as_bytes() {
            scanner
                .push(std::slice::from_ref(byte), &mut output)
                .unwrap();
        }
        let scanner_events = scanner.reservation_events();
        let scanner_capacity = scanner.requested_capacity();
        scanner.finish(&mut output).unwrap();
        assert_eq!(output, format!("passwd={MARKER};").repeat(4_096).as_bytes());
        assert!(scanner_events <= 32, "reservation events: {scanner_events}");
        assert!(scanner_capacity <= relative_bound(input.len()).unwrap());

        let mut fields = Map::new();
        for index in 0..4_096 {
            fields.insert(format!("field_{index}"), Value::String("passwd=x;".into()));
        }
        let input = Value::Object(fields);
        let mut original = CountingWriter::default();
        serde_json::to_writer(&mut original, &input).unwrap();
        let limit = relative_bound(original.bytes).unwrap();
        let projected = project_value(input).unwrap();
        let mut writer = BoundedWriter::new(limit);
        serde_json::to_writer(&mut writer, &projected).unwrap();
        assert!(writer.reservation_events <= 32);
        assert!(writer.requested_capacity <= limit);
        assert!(writer.bytes.len() <= limit);
    }

    #[test]
    fn cleared_stream_buffer_keeps_current_capacity_and_cumulative_bounds() {
        let mut scanner = Scanner::new();
        let mut current = Vec::new();
        let mut maximum_capacity = 0;
        for _ in 0..32_768 {
            scanner.push(b"password=x;", &mut current).unwrap();
            maximum_capacity = maximum_capacity.max(current.capacity());
            current.clear();
        }
        scanner.finish(&mut current).unwrap();
        assert!(maximum_capacity <= relative_bound(b"password=x;".len()).unwrap());
        assert_eq!(current, Vec::<u8>::new());
    }

    #[test]
    fn arbitrary_stream_bytes_and_repeated_expansion_stay_bounded() {
        let input = b"\xff api_key=opaque\x80 tail";
        let output = streamed(input, &(0..=input.len()).collect::<Vec<_>>()).unwrap();
        assert_eq!(
            output,
            [b"\xff api_key=".as_slice(), MARKER.as_bytes(), b" tail"].concat()
        );

        let input = "api_key=x;".repeat(4_096);
        let projected = text(&input).unwrap();
        assert!(projected.len() <= input.len() * 6 + 256);
        assert_eq!(projected.matches(MARKER).count(), 4_096);
    }

    #[test]
    fn head_tail_tool_truncation_keeps_visible_markers_whole() {
        let limit = 128;
        let head = format!("head-{MARKER}{}", "x".repeat(256));
        let head_result = truncate_tool_output(&head, limit);
        assert!(head_result.len() <= limit);
        assert!(head_result.starts_with(&format!("head-{MARKER}")));
        assert!(head_result.contains(TRUNCATED));
        assert!(!head_result.ends_with(TRUNCATED));

        let tail = format!("{}{}tail", "x".repeat(256), MARKER);
        let tail_result = truncate_tool_output(&tail, limit);
        assert!(tail_result.len() <= limit);
        assert!(tail_result.ends_with(&format!("{MARKER}tail")));
        assert!(tail_result.contains(TRUNCATED));

        let assert_complete_markers = |result: &str| {
            assert!(result.len() <= limit);
            for (index, _) in result.match_indices('[') {
                let suffix = &result[index..];
                assert!(
                    suffix.starts_with(MARKER) || suffix.starts_with(TRUNCATED),
                    "partial marker in {result:?}"
                );
            }
            let without_markers = result.replace(MARKER, "").replace(TRUNCATED, "");
            assert!(
                !without_markers.contains(['[', ']']),
                "partial marker in {result:?}"
            );
            assert!(result.contains(TRUNCATED));
        };
        let head_cut = (limit - TRUNCATED.len()) / 2;
        for cut in 1..MARKER.len() {
            let source = format!(
                "{}{}{}",
                "x".repeat(head_cut - cut),
                MARKER,
                "tail".repeat(64)
            );
            assert_complete_markers(&truncate_tool_output(&source, limit));
        }
        let tail_cut = limit - TRUNCATED.len() - head_cut;
        for cut in 1..MARKER.len() {
            let suffix = "t".repeat(tail_cut - MARKER.len() + cut);
            let source = format!("{}{}{}", "x".repeat(256), MARKER, suffix);
            assert_complete_markers(&truncate_tool_output(&source, limit));
        }
        let adjacent = format!("{}{}tail", "x".repeat(256), MARKER.repeat(2));
        assert_complete_markers(&truncate_tool_output(&adjacent, limit));
        let unicode = format!("{}:TAIL", "🪶".repeat(128));
        let mut unicode_projection = super::StreamingProjection::new(limit);
        for chunk in unicode.as_bytes().chunks(1) {
            unicode_projection.push(chunk).unwrap();
        }
        let unicode_result = unicode_projection.finish().unwrap();
        assert!(unicode_result.len() <= limit);
        assert!(unicode_result.ends_with(":TAIL"));
        assert!(!unicode_result.contains('?'));
        assert!(unicode_result.contains(TRUNCATED));

        let exact = "openai_api_key=sk-proj-abcdefghijklmnop0123456789\nordinary";
        assert_eq!(
            text(exact).unwrap(),
            format!("openai_api_key={MARKER}\nordinary")
        );
        let mut exact_projection = super::StreamingProjection::new(256);
        for chunk in exact.as_bytes().chunks(7) {
            exact_projection.push(chunk).unwrap();
        }
        assert_eq!(
            exact_projection.finish().unwrap(),
            format!("openai_api_key={MARKER}\nordinary")
        );

        let crossing_secret = format!("{} sk-abcdefghijklmnop:TAIL", "x".repeat(200));
        let mut crossing_projection = super::StreamingProjection::new(limit);
        for chunk in crossing_secret.as_bytes().chunks(3) {
            crossing_projection.push(chunk).unwrap();
        }
        let crossing_result = crossing_projection.finish().unwrap();
        assert!(crossing_result.contains(MARKER));
        assert!(!crossing_result.contains("sk-abcdefghijklmnop"));
        assert!(crossing_result.ends_with(&format!(" {MARKER}:TAIL")));
        assert!(crossing_result.contains(TRUNCATED));

        let mut projection = super::StreamingProjection::new(limit);
        let streamed = format!("HEAD: api_key=opaque {}:TAIL", "x".repeat(256));
        for chunk in streamed.as_bytes().chunks(7) {
            projection.push(chunk).unwrap();
        }
        let result = projection.finish().unwrap();
        assert!(result.len() <= limit);
        assert!(result.starts_with("HEAD: api_key="));
        assert!(result.contains(MARKER));
        assert!(result.contains(TRUNCATED));
        assert!(result.ends_with(":TAIL"));
    }

    #[test]
    fn head_tail_sink_never_buffers_a_large_single_append() {
        let limit = 128;
        let mut sink = super::HeadTail::new(limit);
        let input = vec![b'x'; 2 * 1024 * 1024];
        super::ProjectionSink::append(&mut sink, &input, input.len(), 0, usize::MAX, false)
            .unwrap();
        assert!(sink.prefix.len() <= sink.prefix_limit());
        assert!(sink.tail.len() <= sink.tail_limit());
        assert!(sink.tail.capacity() <= sink.tail_limit().next_power_of_two());
        let result = sink.into_text();
        assert!(result.len() <= limit);
        assert!(result.starts_with('x'));
        assert!(result.ends_with('x'));
        assert!(result.contains(TRUNCATED));

        let markers = format!("{}tail", MARKER.repeat(10_000));
        let result = truncate_tool_output(&markers, limit);
        assert!(result.len() <= limit);
        assert!(result.ends_with("tail"));
        assert!(result.contains(TRUNCATED));
    }

    #[test]
    fn tiny_tool_budgets_omit_markers_wholly_and_keep_old_indication() {
        let input = format!("{MARKER}tail");
        for limit in 0..input.len() {
            let result = truncate_tool_output(&input, limit);
            assert!(result.len() <= limit);
            assert!(!result.contains(MARKER));
            assert!(!MARKER.starts_with(&result) || result.is_empty());
        }
        assert_eq!(truncate_tool_output("abcdef", 2), "..");
        assert_eq!(truncate_tool_output("kept", 4), "kept");
    }
}
