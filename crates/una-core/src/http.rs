//! URLs and `http:` grants (`spec/24 §2`): which origins and methods a run may reach. Pure, so
//! the checker, the manifest and both engines read grants alike; the client is `una-http`.

/// A method of `Http`, one per operation (`spec/24 §1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Method {
    /// `get`.
    Get,
    /// `post`.
    Post,
    /// `put`.
    Put,
    /// `delete`.
    Delete,
}

impl Method {
    /// The method of an operation's name.
    pub fn of(name: &str) -> Option<Method> {
        Some(match name {
            "get" => Method::Get,
            "post" => Method::Post,
            "put" => Method::Put,
            "delete" => Method::Delete,
            _ => return None,
        })
    }

    /// The name of the operation of `Http` for the method.
    pub fn op(self) -> &'static str {
        match self {
            Method::Get => "get",
            Method::Post => "post",
            Method::Put => "put",
            Method::Delete => "delete",
        }
    }

    /// The method as a request line writes it.
    pub fn token(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

/// An absolute `http` or `https` URL, as a request needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    /// Whether the scheme is `https`.
    pub tls: bool,
    /// The host, in lowercase: a name, an IPv4 address, or an IPv6 address in brackets.
    pub host: String,
    /// The port, the scheme's default when the URL names none.
    pub port: u16,
    /// The path and query, at least `/`; the fragment is never sent.
    pub target: String,
}

impl Url {
    /// The URL of `text`, or `None` when it is not an absolute `http` or `https` URL with no
    /// user info, whose host and target hold only characters a request may carry (`BadUrl`).
    pub fn parse(text: &str) -> Option<Url> {
        let (tls, rest) = match strip_prefix_ignore_case(text, "https://") {
            Some(rest) => (true, rest),
            None => (false, strip_prefix_ignore_case(text, "http://")?),
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, target) = rest.split_at(end);
        if authority.contains('@') {
            return None;
        }
        let (host, port) = host_and_port(authority, tls)?;
        let target = target.split('#').next().unwrap_or_default();
        if !target.bytes().all(|b| b.is_ascii_graphic()) {
            return None;
        }
        let target = match target {
            "" => "/".to_owned(),
            t if t.starts_with('?') => format!("/{t}"),
            t => t.to_owned(),
        };
        Some(Url {
            tls,
            host,
            port,
            target,
        })
    }

    /// The `Host` header of the URL: the host, and the port when it is not the default.
    pub fn host_header(&self) -> String {
        if self.port == default_port(self.tls) {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

fn strip_prefix_ignore_case<'t>(text: &'t str, prefix: &str) -> Option<&'t str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| text.get(prefix.len()..))
        .flatten()
}

fn default_port(tls: bool) -> u16 {
    if tls { 443 } else { 80 }
}

/// The host, lowercase, and the port of an authority.
fn host_and_port(authority: &str, tls: bool) -> Option<(String, u16)> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let close = rest.find(']')?;
        let (inside, after) = rest.split_at(close);
        let after = after.get(1..).unwrap_or_default();
        if inside.is_empty()
            || !inside
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
        {
            return None;
        }
        (format!("[{}]", inside.to_ascii_lowercase()), after)
    } else {
        let colon = authority.find(':').unwrap_or(authority.len());
        let (host, after) = authority.split_at(colon);
        let label = |l: &str| {
            !l.is_empty()
                && l.len() <= 63
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !l.starts_with('-')
                && !l.ends_with('-')
        };
        if host.is_empty() || host.len() > 253 || !host.split('.').all(label) {
            return None;
        }
        (host.to_ascii_lowercase(), after)
    };
    let port = match port {
        "" => default_port(tls),
        p => {
            let digits = p.strip_prefix(':')?;
            if digits.is_empty() || digits.len() > 5 || !digits.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            match digits.parse::<u16>() {
                Ok(0) | Err(_) => return None,
                Ok(port) => port,
            }
        }
    };
    Some((host, port))
}

/// The period of a rate limit (ADR 0117 rule 8, `spec/27 §9`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Period {
    /// `per second`, `/second`.
    Second,
    /// `per minute`, `/minute`.
    Minute,
    /// `per hour`, `/hour`.
    Hour,
}

impl Period {
    /// The period's word, as both forms write it.
    pub fn word(self) -> &'static str {
        match self {
            Period::Second => "second",
            Period::Minute => "minute",
            Period::Hour => "hour",
        }
    }

    /// The period of a word, or `None`.
    pub fn of(word: &str) -> Option<Period> {
        Some(match word {
            "second" => Period::Second,
            "minute" => Period::Minute,
            "hour" => Period::Hour,
            _ => return None,
        })
    }

    /// The period in nanoseconds.
    pub fn nanos(self) -> u64 {
        match self {
            Period::Second => 1_000_000_000,
            Period::Minute => 60_000_000_000,
            Period::Hour => 3_600_000_000_000,
        }
    }
}

/// The largest count of a limit, 2^31 − 1 (ADR 0117 rule 8).
pub const MAX_LIMIT: u32 = 0x7fff_ffff;

/// The queue of a grant without `waiting` (ADR 0117 rule 8, Defaults).
pub const DEFAULT_WAITING: u32 = 1024;

/// The limits of an `Http` grant (ADR 0117 rule 8, `spec/27 §9`): each kind at most once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    /// At most `n` requests per period.
    pub rate: Option<(u32, Period)>,
    /// At most `n` requests in flight.
    pub at_once: Option<u32>,
    /// At most `n` requests waiting; [`DEFAULT_WAITING`] without it.
    pub waiting: Option<u32>,
}

impl Limits {
    /// Whether the grant carries any limit.
    pub fn any(&self) -> bool {
        self.rate.is_some() || self.at_once.is_some() || self.waiting.is_some()
    }

    /// Whether a request can ever have to wait at the grant: a rate or a count in flight.
    pub fn binding(&self) -> bool {
        self.rate.is_some() || self.at_once.is_some()
    }

    /// How many requests may wait at the grant.
    pub fn queue(&self) -> u32 {
        self.waiting.unwrap_or(DEFAULT_WAITING)
    }

    /// The suffixes of the text form, in the order rate, at once, waiting: `;100/minute`,
    /// `;8/once`, `;waiting=1000`.
    pub fn suffix(&self) -> String {
        let mut out = String::new();
        if let Some((count, period)) = self.rate {
            out.push_str(&format!(";{count}/{}", period.word()));
        }
        if let Some(count) = self.at_once {
            out.push_str(&format!(";{count}/once"));
        }
        if let Some(count) = self.waiting {
            out.push_str(&format!(";waiting={count}"));
        }
        out
    }
}

/// Why the limits of a grant are invalid (ADR 0117 rule 8, `spec/27 §9` rule 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LimitError {
    /// A limit that is none of the forms.
    Form,
    /// A count below 1 or above [`MAX_LIMIT`].
    Count,
    /// A kind of limit given twice; two rates are one kind.
    Twice,
}

impl LimitError {
    /// The error as a message says it.
    pub fn message(&self) -> &'static str {
        match self {
            LimitError::Form => {
                "a limit is `<n>/second`, `<n>/minute`, `<n>/hour`, `<n>/once` or `waiting=<n>`"
            }
            LimitError::Count => "the count of a limit is from 1 to 2147483647",
            LimitError::Twice => "a kind of limit is given at most once on a grant",
        }
    }
}

/// A count of a limit: decimal digits, from 1 to [`MAX_LIMIT`].
pub fn limit_count(digits: &str) -> Result<u32, LimitError> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(LimitError::Form);
    }
    match digits.parse::<u64>() {
        Ok(n) if (1..=u64::from(MAX_LIMIT)).contains(&n) => {
            u32::try_from(n).map_err(|_| LimitError::Count)
        }
        _ => Err(LimitError::Count),
    }
}

impl Limits {
    /// The limits of the text form's suffixes, each without its `;`.
    pub fn parse<'s>(parts: impl IntoIterator<Item = &'s str>) -> Result<Limits, LimitError> {
        let mut limits = Limits::default();
        for part in parts {
            if let Some(count) = part.strip_prefix("waiting=") {
                let count = limit_count(count)?;
                if limits.waiting.replace(count).is_some() {
                    return Err(LimitError::Twice);
                }
                continue;
            }
            let (count, unit) = part.split_once('/').ok_or(LimitError::Form)?;
            if unit == "once" {
                let count = limit_count(count)?;
                if limits.at_once.replace(count).is_some() {
                    return Err(LimitError::Twice);
                }
                continue;
            }
            let period = Period::of(unit).ok_or(LimitError::Form)?;
            let count = limit_count(count)?;
            if limits.rate.replace((count, period)).is_some() {
                return Err(LimitError::Twice);
            }
        }
        Ok(limits)
    }
}

/// A grant `http:<origin>` or `http:<origin>:<methods>` (`spec/24 §2` rule 1), with its
/// limits in edition `0.4` (`spec/27 §9`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    tls: bool,
    /// The host, lowercase; `*.` first for one label of wildcard.
    host: String,
    port: u16,
    methods: Vec<Method>,
    limits: Limits,
}

impl Grant {
    /// The grant of a spec without limits, or `None` when it is not one: a spec with limit
    /// suffixes is not one, as `una.toml` reads grants (`spec/24 §2`).
    pub fn parse(spec: &str) -> Option<Grant> {
        if spec.contains(';') {
            return None;
        }
        Grant::parse_origin(spec)
    }

    /// The grant of a spec with optional limit suffixes, as `ita.una` and `--grant` give them
    /// in edition `0.4` (`spec/27 §9`): `None` when the spec without its suffixes is not a
    /// grant, and the error of its limits when they are invalid.
    pub fn parse_limited(spec: &str) -> Option<Result<Grant, LimitError>> {
        let (grant, suffixes) = match spec.split_once(';') {
            Some((grant, suffixes)) => (grant, Some(suffixes)),
            None => (spec, None),
        };
        let mut parsed = Grant::parse_origin(grant)?;
        if let Some(suffixes) = suffixes {
            match Limits::parse(suffixes.split(';')) {
                Ok(limits) => parsed.limits = limits,
                Err(error) => return Some(Err(error)),
            }
        }
        Some(Ok(parsed))
    }

    /// The limits of the grant.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The grant of a spec without suffixes.
    fn parse_origin(spec: &str) -> Option<Grant> {
        let rest = spec.strip_prefix("http:")?;
        let (tls, rest) = match rest.strip_prefix("https://") {
            Some(rest) => (true, rest),
            None => (false, rest.strip_prefix("http://")?),
        };
        // The host, in brackets for IPv6, whose colons are its own; then `:<port>` when
        // digits follow, then `:<methods>`.
        let end = if rest.starts_with('[') {
            rest.find(']')? + 1
        } else {
            rest.find(':').unwrap_or(rest.len())
        };
        let (host, after) = rest.split_at(end);
        let parts = match after {
            "" => None,
            after => Some(after.strip_prefix(':')?.splitn(2, ':')),
        };
        let mut port_text = None;
        let mut methods_text = None;
        for part in parts.into_iter().flatten() {
            if port_text.is_none()
                && methods_text.is_none()
                && !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
            {
                port_text = Some(part);
            } else if methods_text.is_none() {
                methods_text = Some(part);
            } else {
                return None;
            }
        }
        let (pattern, check) = match host.strip_prefix("*.") {
            Some(rest) => (format!("*.{}", rest.to_ascii_lowercase()), rest),
            None => (host.to_ascii_lowercase(), host),
        };
        let authority = match port_text {
            Some(port) => format!("{check}:{port}"),
            None => check.to_owned(),
        };
        let (_, port) = host_and_port(&authority, tls)?;
        let methods = match methods_text {
            None => vec![Method::Get],
            Some(list) => {
                let mut methods = Vec::new();
                for name in list.split(',') {
                    let method = Method::of(name)?;
                    if !methods.contains(&method) {
                        methods.push(method);
                    }
                }
                methods.sort_unstable();
                methods
            }
        };
        Some(Grant {
            tls,
            host: pattern,
            port,
            methods,
            limits: Limits::default(),
        })
    }

    /// The methods the grant allows, in order.
    pub fn methods(&self) -> &[Method] {
        &self.methods
    }

    /// Whether the grant allows `method` on `url` (`spec/24 §2` rule 2).
    pub fn allows(&self, url: &Url, method: Method) -> bool {
        if self.tls != url.tls || self.port != url.port || !self.methods.contains(&method) {
            return false;
        }
        match self.host.strip_prefix("*.") {
            Some(suffix) => url
                .host
                .split_once('.')
                .is_some_and(|(first, rest)| !first.is_empty() && rest == suffix),
            None => self.host == url.host,
        }
    }
}

/// Whether some grant of `grants` allows `method` on `url`.
pub fn allowed(grants: &[Grant], url: &Url, method: Method) -> bool {
    grants.iter().any(|grant| grant.allows(url, method))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn urls_parse_strictly() {
        let u = url("HTTPS://Api.Example.com/v1/items?q=1#frag");
        assert_eq!(
            (u.tls, u.host.as_str(), u.port, u.target.as_str()),
            (true, "api.example.com", 443, "/v1/items?q=1")
        );
        assert_eq!(url("http://127.0.0.1:8080").target, "/");
        assert_eq!(url("http://h?x=1").target, "/?x=1");
        assert_eq!(url("http://[::1]:9/").host, "[::1]");
        assert_eq!(url("http://h:8080/").host_header(), "h:8080");
        assert_eq!(url("https://h:443/").host_header(), "h");
        for bad in [
            "ftp://h/",
            "h/",
            "http://",
            "http://user:pw@h/",
            "http://h:0/",
            "http://h:99999/",
            "http://h:/",
            "http://-h/",
            "http://h/a b",
            "http://h/é",
            "http://h_/",
            "http://[zz]/",
        ] {
            assert_eq!(Url::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn grants_allow_their_origins_and_methods_only() {
        let api = Grant::parse("http:https://api.example.com:get,post").unwrap();
        assert!(api.allows(&url("https://api.example.com/x"), Method::Post));
        assert!(!api.allows(&url("https://api.example.com/x"), Method::Delete));
        assert!(!api.allows(&url("http://api.example.com/x"), Method::Get));
        assert!(!api.allows(&url("https://api.example.com:8443/x"), Method::Get));
        assert!(!api.allows(&url("https://evil.api.example.com/"), Method::Get));
        let local = Grant::parse("http:http://127.0.0.1:8080").unwrap();
        assert!(local.allows(&url("http://127.0.0.1:8080/"), Method::Get));
        assert!(!local.allows(&url("http://127.0.0.1:8080/"), Method::Post));
        assert!(!local.allows(&url("http://127.0.0.1/"), Method::Get));
        let ported = Grant::parse("http:http://localhost:9000:put,delete").unwrap();
        assert!(ported.allows(&url("http://localhost:9000/"), Method::Delete));
        let wild = Grant::parse("http:https://*.example.com").unwrap();
        assert!(wild.allows(&url("https://a.example.com/"), Method::Get));
        assert!(!wild.allows(&url("https://example.com/"), Method::Get));
        assert!(!wild.allows(&url("https://a.b.example.com/"), Method::Get));
        // An IPv6 host in brackets, with and without a port and methods.
        let six = Grant::parse("http:http://[::1]:18083").unwrap();
        assert!(six.allows(&url("http://[::1]:18083/"), Method::Get));
        assert!(!six.allows(&url("http://[::1]/"), Method::Get));
        assert!(!six.allows(&url("http://127.0.0.1:18083/"), Method::Get));
        let bare = Grant::parse("http:https://[2001:DB8::1]").unwrap();
        assert!(bare.allows(&url("https://[2001:db8::1]/"), Method::Get));
        let both = Grant::parse("http:http://[::1]:8080:get,post").unwrap();
        assert!(both.allows(&url("http://[::1]:8080/"), Method::Post));
        let methods = Grant::parse("http:http://[::1]:delete").unwrap();
        assert!(methods.allows(&url("http://[::1]/"), Method::Delete));
        assert!(!methods.allows(&url("http://[::1]/"), Method::Get));
        for bad in [
            "http:http://[::1",
            "http:http://[]",
            "http:http://[::1]x",
            "http:http://[::1]:",
            "http:http://[::1]:80:get:x",
            "http:http://[zz]:80",
        ] {
            assert_eq!(Grant::parse(bad), None, "{bad}");
        }
        for bad in [
            "http:ftp://h",
            "https://h",
            "http:https://h:patch",
            "http:https://h:80:get:x",
            "http:https://user@h",
            "http:https://*",
            "http:https://h:0",
        ] {
            assert_eq!(Grant::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn limits_parse_in_the_text_form_only_where_asked() {
        let spec = "http:https://api.example.com:get,post;100/minute;8/once;waiting=1000";
        let grant = Grant::parse_limited(spec).unwrap().unwrap();
        assert_eq!(
            *grant.limits(),
            Limits {
                rate: Some((100, Period::Minute)),
                at_once: Some(8),
                waiting: Some(1000),
            }
        );
        assert_eq!(grant.limits().suffix(), ";100/minute;8/once;waiting=1000");
        assert!(grant.allows(&url("https://api.example.com/x"), Method::Post));
        // In any order; without suffixes, no limits.
        let reordered = Grant::parse_limited("http:http://[::1]:8080;waiting=3;2/second").unwrap();
        assert_eq!(reordered.unwrap().limits().rate, Some((2, Period::Second)));
        let plain = Grant::parse_limited("http:https://h").unwrap().unwrap();
        assert!(!plain.limits().any());
        // `una.toml` reads no limits (U7002 there).
        assert_eq!(Grant::parse(spec), None);
        assert_eq!(Grant::parse("http:https://h;1/once"), None);
        // Errors of the limits, and of the grant before them.
        let error = |spec: &str| Grant::parse_limited(spec).unwrap().unwrap_err();
        assert_eq!(error("http:https://h;0/second"), LimitError::Count);
        assert_eq!(error("http:https://h;2147483648/once"), LimitError::Count);
        assert_eq!(error("http:https://h;waiting=0"), LimitError::Count);
        assert_eq!(error("http:https://h;1/second;2/hour"), LimitError::Twice);
        assert_eq!(error("http:https://h;1/once;1/once"), LimitError::Twice);
        assert_eq!(error("http:https://h;1/week"), LimitError::Form);
        assert_eq!(error("http:https://h;"), LimitError::Form);
        assert_eq!(error("http:https://h;+1/once"), LimitError::Form);
        assert_eq!(error("http:https://h;1 /once"), LimitError::Form);
        assert!(
            Grant::parse_limited("http:https://h;2147483647/once")
                .unwrap()
                .is_ok()
        );
        assert_eq!(Grant::parse_limited("http:ftp://h;1/once"), None);
    }
}
