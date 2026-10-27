//! The server's writer of replies, with no I/O (`spec/31 §5`, ADR 0123 rules 13 and 14).
//!
//! A reply is checked before any byte of it is written, so a header can never inject a line or
//! a second response, and the host writes the framing headers itself. Writing is O(r) for a
//! reply of r bytes, into one buffer reserved once.

/// A reply, as the program hands it to the host: the plain owned data of the program's
/// `Reply` (ADR 0123 rule 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// The status, 200 to 599.
    pub status: u16,
    /// The headers, in order, names in any letter case.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: String,
}

/// Why `reply` fails with `BadReply` (ADR 0123 rule 13). Nothing is sent, and the reply right
/// stays held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadReply {
    /// A status outside 200 to 599.
    Status,
    /// A body on a `204` or a `304`.
    Body,
    /// A header name that is not a token.
    Name,
    /// A header value with CR, LF, NUL or another control character but tab.
    Value,
    /// A header the host writes, or that belongs to the connection.
    Reserved,
    /// More than [`MAX_REPLY_HEADERS`] headers, or more than [`MAX_REPLY_HEADER_BYTES`] of
    /// them.
    TooLarge,
}

/// The most headers of a reply, as many as a response the client reads may have
/// (`spec/24 §4` rule 1).
pub const MAX_REPLY_HEADERS: usize = 100;

/// The most bytes of a reply's headers, each counted as `name: value` with its CRLF.
pub const MAX_REPLY_HEADER_BYTES: usize = 64 * 1024;

/// The headers a reply may not set, in lower case: the host writes them, or they belong to
/// the connection.
pub const RESERVED_HEADERS: [&str; 9] = [
    "connection",
    "content-length",
    "date",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// The content type of a reply that names none and has a body: every body is UTF-8 `Text`.
pub const DEFAULT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

/// The bytes of `100 Continue`, sent once the host admits the body of a request that expects
/// it (rule 10).
pub const CONTINUE: &[u8] = b"HTTP/1.1 100 Continue\r\n\r\n";

/// Whether a status has no body: `204` and `304`.
fn bodiless(status: u16) -> bool {
    status == 204 || status == 304
}

/// The checks of `reply` (ADR 0123 rule 13), each O(1) per byte of the headers.
pub fn check(reply: &Reply) -> Result<(), BadReply> {
    if !(200..=599).contains(&reply.status) {
        return Err(BadReply::Status);
    }
    if bodiless(reply.status) && !reply.body.is_empty() {
        return Err(BadReply::Body);
    }
    if reply.headers.len() > MAX_REPLY_HEADERS {
        return Err(BadReply::TooLarge);
    }
    let mut bytes = 0usize;
    for (name, value) in &reply.headers {
        if !crate::strict::is_token(name.as_bytes()) {
            return Err(BadReply::Name);
        }
        if value.bytes().any(|b| (b < 0x20 && b != b'\t') || b == 0x7f) {
            return Err(BadReply::Value);
        }
        if RESERVED_HEADERS
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
        {
            return Err(BadReply::Reserved);
        }
        bytes = bytes
            .saturating_add(name.len())
            .saturating_add(value.len())
            .saturating_add(4);
        if bytes > MAX_REPLY_HEADER_BYTES {
            return Err(BadReply::TooLarge);
        }
    }
    Ok(())
}

/// How the host writes a reply on its connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Framing {
    /// The request was `HEAD`: the reply goes without its body, with the `Content-Length` of
    /// the body given.
    pub head: bool,
    /// The connection closes after this reply: `Connection: close` is written.
    pub close: bool,
    /// The time of the reply, in seconds since 1970-01-01T00:00:00Z, for `Date`.
    pub date: u64,
}

/// Checks `reply` and writes its bytes to `out` (ADR 0123 rule 13): the status line with its
/// reason phrase; `Date`; `Content-Length` except on a `204` or a `304`; the program's
/// headers, in order; `content-type: text/plain; charset=utf-8` when the reply names no
/// content type and carries a body; `Connection: close` when the connection closes; and the
/// body, except for `HEAD`, `204` and `304`. Nothing is written when a check fails.
pub fn write(reply: &Reply, framing: Framing, out: &mut Vec<u8>) -> Result<(), BadReply> {
    check(reply)?;
    let bodiless = bodiless(reply.status);
    let typed = reply
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"));
    let send_body = !bodiless && !framing.head;
    let headers: usize = reply
        .headers
        .iter()
        .map(|(name, value)| name.len() + value.len() + 4)
        .sum();
    out.reserve(160 + headers + if send_body { reply.body.len() } else { 0 });
    status_line(reply.status, out);
    date_header(framing.date, out);
    if !bodiless {
        out.extend_from_slice(format!("Content-Length: {}\r\n", reply.body.len()).as_bytes());
    }
    for (name, value) in &reply.headers {
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    if !typed && send_body {
        out.extend_from_slice(b"content-type: ");
        out.extend_from_slice(DEFAULT_CONTENT_TYPE.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    if framing.close {
        out.extend_from_slice(b"Connection: close\r\n");
    }
    out.extend_from_slice(b"\r\n");
    if send_body {
        out.extend_from_slice(reply.body.as_bytes());
    }
    Ok(())
}

/// Writes one of the host's own answers (ADR 0123 rules 8, 10, 11 and 14): `status` with an
/// empty body, `Retry-After: 1` when asked, and `Connection: close` when the connection
/// closes, which every refusal of a request does.
pub fn write_answer(status: u16, retry_after: bool, close: bool, date: u64, out: &mut Vec<u8>) {
    status_line(status, out);
    date_header(date, out);
    out.extend_from_slice(b"Content-Length: 0\r\n");
    if retry_after {
        out.extend_from_slice(b"Retry-After: 1\r\n");
    }
    if close {
        out.extend_from_slice(b"Connection: close\r\n");
    }
    out.extend_from_slice(b"\r\n");
}

/// `HTTP/1.1 <status> <reason>` and its CRLF; an empty reason for a status that has none.
fn status_line(status: u16, out: &mut Vec<u8>) {
    out.extend_from_slice(format!("HTTP/1.1 {status} {}\r\n", reason(status)).as_bytes());
}

/// `Date: <IMF-fixdate>` and its CRLF.
fn date_header(date: u64, out: &mut Vec<u8>) {
    out.extend_from_slice(b"Date: ");
    out.extend_from_slice(http_date(date).as_bytes());
    out.extend_from_slice(b"\r\n");
}

/// The reason phrase of a status: RFC 9110 §15, with `429` and `431` of RFC 6585; empty for
/// any other.
pub fn reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        305 => "Use Proxy",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        426 => "Upgrade Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        _ => "",
    }
}

/// An IMF-fixdate (RFC 9110 §5.6.7), such as `Sun, 06 Nov 1994 08:49:37 GMT`, for `seconds`
/// since 1970-01-01T00:00:00Z. O(1).
pub fn http_date(seconds: u64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = seconds / 86_400;
    let time = seconds % 86_400;
    let weekday = DAYS
        .get(usize::try_from(days % 7).unwrap_or(0))
        .copied()
        .unwrap_or("Thu");
    // The civil date of a day count, by the proleptic Gregorian calendar in eras of 400
    // years (H. Hinnant, "chrono-Compatible Low-Level Date Algorithms").
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    let month = MONTHS
        .get(usize::try_from(month - 1).unwrap_or(0))
        .copied()
        .unwrap_or("Jan");
    format!(
        "{weekday}, {day:02} {month} {year:04} {:02}:{:02}:{:02} GMT",
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}

#[cfg(test)]
mod tests;
