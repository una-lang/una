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

/// A grant `http:<origin>` or `http:<origin>:<methods>` (`spec/24 §2` rule 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    tls: bool,
    /// The host, lowercase; `*.` first for one label of wildcard.
    host: String,
    port: u16,
    methods: Vec<Method>,
}

impl Grant {
    /// The grant of a spec, or `None` when it is not one.
    pub fn parse(spec: &str) -> Option<Grant> {
        let rest = spec.strip_prefix("http:")?;
        let (tls, rest) = match rest.strip_prefix("https://") {
            Some(rest) => (true, rest),
            None => (false, rest.strip_prefix("http://")?),
        };
        // The host, in brackets for IPv6, whose colons are its own (N-30); then `:<port>` when
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
        // N-30 #5: an IPv6 host in brackets, with and without a port and methods.
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
}
