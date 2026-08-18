//! `/api` Host / Origin / `sec-fetch-site` trust fence and privileged-method set.

/// Configured `trustedHosts` entry that is not a bare `host` or `host:port` authority.
#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    /// Entry failed WHATWG-equivalent canonical-authority check.
    #[error("trustedHosts entry {entry:?} is not a bare host[:port] authority")]
    InvalidAuthority {
        /// The configured entry, verbatim.
        entry: String,
    },
}

/// Whether a normalized URL hostname names loopback: `localhost`, `[::1]`, or IPv4 127/8.
///
/// `hostname` is a WHATWG hostname (IPv6 keeps brackets). Port is not part of this argument.
#[must_use]
pub fn is_loopback_hostname(hostname: &str) -> bool {
    if hostname == "localhost" || hostname == "[::1]" {
        return true;
    }
    let parts: Vec<&str> = hostname.split('.').collect();
    if parts.len() != 4 || parts[0] != "127" {
        return false;
    }
    parts.iter().all(|part| {
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        part.parse::<u32>().map(|n| n <= 255).unwrap_or(false)
    })
}

/// Assert one configured `trustedHosts` entry is a bare `host` or `host:port`.
///
/// The entry must survive WHATWG-equivalent parse unchanged (lowercase compare). Path, userinfo, whitespace, a dangling colon, and non-canonical hosts such as `0x7f.0.0.1` fail loudly.
pub fn assert_trusted_authority(entry: &str) -> Result<(), TrustError> {
    let invalid = || TrustError::InvalidAuthority {
        entry: entry.to_string(),
    };
    let Some(entry_url) = parse_http_authority(entry) else {
        return Err(invalid());
    };
    let Some(canonical) = canonical_authority(entry, &entry_url) else {
        return Err(invalid());
    };
    if canonical == entry.to_lowercase() {
        Ok(())
    } else {
        Err(invalid())
    }
}

/// Whether one `/api` request may pass the Host / Origin / `sec-fetch-site` fence.
///
/// `host` is the Host header (`host[:port]`). Missing or unparsable Host is untrusted. Host must be loopback or listed in `trusted_hosts`. `sec-fetch-site: cross-site` is refused. Absent Origin is trusted after Host. Origin `"null"` is refused. Origin host must equal Host host (port included when present).
#[must_use]
pub fn is_trusted_api_request(
    host: Option<&str>,
    origin: Option<&str>,
    sec_fetch_site: Option<&str>,
    trusted_hosts: &[String],
) -> bool {
    let Some(host) = host else {
        return false;
    };
    let Some(host_url) = parse_http_authority(host) else {
        return false;
    };
    if !is_loopback_hostname(&host_url.hostname) && !is_trusted_authority(&host_url, trusted_hosts)
    {
        return false;
    }
    if sec_fetch_site == Some("cross-site") {
        return false;
    }
    let Some(origin) = origin else {
        return true;
    };
    let Some(origin_url) = parse_url(origin) else {
        return false;
    };
    origin_url.host == host_url.host
}

/// Whether `method` is in the closed privileged dotted-method set.
#[must_use]
#[allow(clippy::match_like_matches_macro)]
pub fn is_privileged_method(method: &str) -> bool {
    match method {
        "agentPreset.read"
        | "agentPreset.copy"
        | "agentPreset.openDocument"
        | "agentPreset.remove"
        | "host.pickDirectory"
        | "host.openPath"
        | "settings.describe"
        | "settings.openDocument"
        | "settings.update"
        | "settings.replace"
        | "settings.mutate"
        | "credentials.describe"
        | "credentials.set"
        | "credentials.unset"
        | "llm.discoverModels" => true,
        _ => false,
    }
}

/// Whether a Host header parses to a loopback hostname.
///
/// Missing or unparsable Host is `false`. Privileged methods still require this even when `trusted_hosts` would pass the outer fence.
#[must_use]
pub fn privileged_requires_loopback(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    let Some(host_url) = parse_http_authority(host) else {
        return false;
    };
    is_loopback_hostname(&host_url.hostname)
}

#[derive(Clone, Debug)]
struct ParsedUrl {
    hostname: String,
    port: String,
    host: String,
}

#[derive(Clone, Copy)]
enum Scheme {
    Http,
    Https,
}

impl Scheme {
    fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

fn parse_http_authority(authority: &str) -> Option<ParsedUrl> {
    parse_url(&format!("http://{authority}"))
}

fn parse_https_authority(authority: &str) -> Option<ParsedUrl> {
    parse_url(&format!("https://{authority}"))
}

fn canonical_authority(entry: &str, entry_url: &ParsedUrl) -> Option<String> {
    let port = if !entry_url.port.is_empty() {
        entry_url.port.clone()
    } else {
        parse_https_authority(entry)?.port
    };
    if port.is_empty() {
        Some(entry_url.hostname.clone())
    } else {
        Some(format!("{}:{port}", entry_url.hostname))
    }
}

fn is_trusted_authority(host_url: &ParsedUrl, trusted_hosts: &[String]) -> bool {
    trusted_hosts.iter().any(|entry| {
        let Some(entry_url) = parse_http_authority(entry) else {
            return false;
        };
        let Some(canonical) = canonical_authority(entry, &entry_url) else {
            return false;
        };
        if canonical == entry_url.hostname {
            entry_url.hostname == host_url.hostname
        } else {
            entry_url.host == host_url.host
        }
    })
}

fn preprocess(input: &str) -> String {
    let stripped: String = input
        .chars()
        .filter(|c| *c != '\t' && *c != '\n' && *c != '\r')
        .collect();
    let bytes = stripped.as_bytes();
    let start = bytes.iter().position(|b| *b > 0x20).unwrap_or(bytes.len());
    let end = bytes.iter().rposition(|b| *b > 0x20).map_or(0, |i| i + 1);
    if start >= end {
        String::new()
    } else {
        stripped[start..end].to_string()
    }
}

fn parse_url(input: &str) -> Option<ParsedUrl> {
    let processed = preprocess(input);
    let colon = processed.find(':')?;
    let scheme = match processed[..colon].to_ascii_lowercase().as_str() {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        _ => return None,
    };
    let rest = &processed[colon + 1..];
    if !rest.starts_with("//") {
        return None;
    }
    let rest = &rest[2..];
    let auth_end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..auth_end];
    if authority.is_empty() {
        return None;
    }
    let hostport = match authority.rfind('@') {
        Some(i) => &authority[i + 1..],
        None => authority,
    };
    let (hostname, port_num) = parse_host_and_port(hostport)?;
    let displayed_port = match port_num {
        Some(p) if p == scheme.default_port() => String::new(),
        Some(p) => p.to_string(),
        None => String::new(),
    };
    let host = if displayed_port.is_empty() {
        hostname.clone()
    } else {
        format!("{hostname}:{displayed_port}")
    };
    Some(ParsedUrl {
        hostname,
        port: displayed_port,
        host,
    })
}

fn parse_host_and_port(hostport: &str) -> Option<(String, Option<u16>)> {
    if let Some(inner) = hostport.strip_prefix('[') {
        let close = inner.find(']')?;
        let ipv6 = parse_ipv6(&inner[..close])?;
        let rest = &inner[close + 1..];
        let hostname = format!("[{}]", serialize_ipv6(ipv6));
        let port = parse_optional_port(rest)?;
        return Some((hostname, port));
    }
    let (host_raw, port) = match hostport.find(':') {
        Some(i) => (&hostport[..i], parse_optional_port(&hostport[i..])?),
        None => (hostport, None),
    };
    if host_raw.is_empty() {
        return None;
    }
    let hostname = normalize_domain_or_ipv4(host_raw)?;
    Some((hostname, port))
}

fn parse_optional_port(rest: &str) -> Option<Option<u16>> {
    if rest.is_empty() {
        return Some(None);
    }
    let digits = rest.strip_prefix(':')?;
    if digits.is_empty() {
        return Some(None);
    }
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: u32 = digits.parse().ok()?;
    if value > 65535 {
        return None;
    }
    Some(Some(value as u16))
}

fn normalize_domain_or_ipv4(raw: &str) -> Option<String> {
    let decoded = percent_decode_host(raw)?;
    let lower = decoded.to_ascii_lowercase();
    if ends_in_a_number(&lower) {
        Some(serialize_ipv4(parse_ipv4_host(&lower)?))
    } else if is_valid_domain(&lower) {
        Some(lower)
    } else {
        None
    }
}

fn percent_decode_host(input: &str) -> Option<String> {
    if !input.as_bytes().contains(&b'%') {
        return Some(input.to_string());
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let high = hex_val(bytes[i + 1])?;
            let low = hex_val(bytes[i + 2])?;
            let decoded = (high << 4) | low;
            if decoded == 0 {
                return None;
            }
            out.push(decoded);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn ends_in_a_number(host: &str) -> bool {
    let mut parts: Vec<&str> = host.split('.').collect();
    if parts.last() == Some(&"") {
        parts.pop();
    }
    let Some(last) = parts.last() else {
        return false;
    };
    if last.is_empty() {
        return false;
    }
    if last.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    parse_ipv4_number(last).is_some()
}

fn is_valid_domain(host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    host.chars().all(|c| {
        !matches!(
            c,
            '\0'..=' '
                | '"'
                | '#'
                | '%'
                | '/'
                | ':'
                | '<'
                | '>'
                | '?'
                | '@'
                | '['
                | '\\'
                | ']'
                | '^'
                | '|'
        )
    })
}

fn parse_ipv4_number(input: &str) -> Option<u64> {
    if input.is_empty() {
        return None;
    }
    let (radix, digits) = if let Some(rest) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        (16, rest)
    } else if input.len() > 1 && input.starts_with('0') {
        (8, &input[1..])
    } else {
        (10, input)
    };
    if digits.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(digits, radix).ok()
}

fn parse_ipv4_host(input: &str) -> Option<u32> {
    let mut parts: Vec<&str> = input.split('.').collect();
    if parts.last() == Some(&"") && parts.len() > 1 {
        parts.pop();
    }
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut numbers = Vec::with_capacity(parts.len());
    for part in &parts {
        numbers.push(parse_ipv4_number(part)?);
    }
    let last = *numbers.last()?;
    let max_last = 256u64.pow((5 - numbers.len()) as u32);
    if last >= max_last {
        return None;
    }
    if numbers.iter().take(numbers.len() - 1).any(|n| *n > 255) {
        return None;
    }
    let mut ipv4 = last;
    for (counter, n) in numbers.iter().take(numbers.len() - 1).enumerate() {
        ipv4 += *n * 256u64.pow((3 - counter) as u32);
    }
    u32::try_from(ipv4).ok()
}

fn serialize_ipv4(ipv4: u32) -> String {
    format!(
        "{}.{}.{}.{}",
        (ipv4 >> 24) & 0xff,
        (ipv4 >> 16) & 0xff,
        (ipv4 >> 8) & 0xff,
        ipv4 & 0xff
    )
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn parse_ipv6(input: &str) -> Option<[u16; 8]> {
    let bytes = input.as_bytes();
    let mut address = [0u16; 8];
    let mut piece_index: usize = 0;
    let mut compress: Option<usize> = None;
    let mut pointer = 0usize;
    if bytes.first().copied() == Some(b':') {
        if bytes.get(1).copied() != Some(b':') {
            return None;
        }
        pointer = 2;
        piece_index = 1;
        compress = Some(1);
    }
    while pointer < bytes.len() {
        if piece_index == 8 {
            return None;
        }
        if bytes[pointer] == b':' {
            if compress.is_some() {
                return None;
            }
            pointer += 1;
            piece_index += 1;
            compress = Some(piece_index);
            continue;
        }
        let mut value: u32 = 0;
        let mut length = 0usize;
        while length < 4 && pointer < bytes.len() && bytes[pointer].is_ascii_hexdigit() {
            let digit = u32::from(hex_val(bytes[pointer])?);
            value = value * 16 + digit;
            pointer += 1;
            length += 1;
        }
        if pointer < bytes.len() && bytes[pointer] == b'.' {
            if length == 0 || piece_index > 6 {
                return None;
            }
            pointer -= length;
            let ipv4 = parse_ipv4_in_ipv6(&bytes[pointer..])?;
            address[piece_index] = ((ipv4 >> 16) & 0xffff) as u16;
            address[piece_index + 1] = (ipv4 & 0xffff) as u16;
            piece_index += 2;
            break;
        }
        if pointer < bytes.len() && bytes[pointer] == b':' {
            pointer += 1;
            if pointer == bytes.len() {
                return None;
            }
        } else if pointer != bytes.len() {
            return None;
        }
        address[piece_index] = value as u16;
        piece_index += 1;
    }
    match compress {
        Some(compress_at) => {
            let mut swaps = piece_index - compress_at;
            let mut idx = 7usize;
            while idx != 0 && swaps > 0 {
                address.swap(idx, compress_at + swaps - 1);
                idx -= 1;
                swaps -= 1;
            }
        }
        None => {
            if piece_index != 8 {
                return None;
            }
        }
    }
    Some(address)
}

fn parse_ipv4_in_ipv6(input: &[u8]) -> Option<u32> {
    let mut parts = [0u8; 4];
    let mut part_idx = 0;
    let mut i = 0;
    while part_idx < 4 {
        if i >= input.len() || !input[i].is_ascii_digit() {
            return None;
        }
        let mut value: u32 = 0;
        let start = i;
        while i < input.len() && input[i].is_ascii_digit() {
            let d = u32::from(input[i] - b'0');
            if i > start && value == 0 {
                return None;
            }
            value = value * 10 + d;
            if value > 255 {
                return None;
            }
            i += 1;
        }
        parts[part_idx] = value as u8;
        part_idx += 1;
        if part_idx < 4 {
            if i >= input.len() || input[i] != b'.' {
                return None;
            }
            i += 1;
        }
    }
    if i != input.len() {
        return None;
    }
    Some(
        (u32::from(parts[0]) << 24)
            | (u32::from(parts[1]) << 16)
            | (u32::from(parts[2]) << 8)
            | u32::from(parts[3]),
    )
}

fn longest_zero_run(address: [u16; 8]) -> Option<usize> {
    let mut best_start = 0usize;
    let mut best_len = 0usize;
    let mut i = 0usize;
    while i < 8 {
        if address[i] == 0 {
            let start = i;
            while i < 8 && address[i] == 0 {
                i += 1;
            }
            let len = i - start;
            if len > best_len {
                best_start = start;
                best_len = len;
            }
        } else {
            i += 1;
        }
    }
    if best_len < 2 { None } else { Some(best_start) }
}

fn serialize_ipv6(address: [u16; 8]) -> String {
    let compress = longest_zero_run(address);
    let mut output = String::new();
    let mut ignore0 = false;
    let mut piece_index = 0usize;
    while piece_index < 8 {
        if ignore0 && address[piece_index] == 0 {
            piece_index += 1;
            continue;
        }
        ignore0 = false;
        if compress == Some(piece_index) {
            if piece_index == 0 {
                output.push_str("::");
            } else {
                output.push(':');
            }
            ignore0 = true;
            piece_index += 1;
            continue;
        }
        output.push_str(&format!("{:x}", address[piece_index]));
        if piece_index != 7 {
            output.push(':');
        }
        piece_index += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        assert_trusted_authority, is_loopback_hostname, is_privileged_method,
        is_trusted_api_request, privileged_requires_loopback,
    };

    #[test]
    fn loopback_accepts_localhost_v6_and_127_8() {
        assert!(is_loopback_hostname("localhost"));
        assert!(is_loopback_hostname("[::1]"));
        assert!(is_loopback_hostname("127.0.0.1"));
        assert!(is_loopback_hostname("127.255.255.255"));
        assert!(!is_loopback_hostname("::1"));
        assert!(!is_loopback_hostname("128.0.0.1"));
        assert!(!is_loopback_hostname("example.com"));
    }

    #[test]
    fn missing_host_is_untrusted() {
        assert!(!is_trusted_api_request(None, None, None, &[]));
    }

    #[test]
    fn cross_site_fetch_is_rejected() {
        assert!(!is_trusted_api_request(
            Some("127.0.0.1:3080"),
            Some("http://127.0.0.1:3080"),
            Some("cross-site"),
            &[],
        ));
    }

    #[test]
    fn origin_null_is_rejected() {
        assert!(!is_trusted_api_request(
            Some("127.0.0.1:3080"),
            Some("null"),
            None,
            &[],
        ));
    }

    #[test]
    fn trusted_host_allows_non_loopback_host_header() {
        assert_trusted_authority("harness.internal").unwrap();
        assert!(is_trusted_api_request(
            Some("harness.internal"),
            None,
            None,
            &["harness.internal".into()],
        ));
    }

    #[test]
    fn malformed_trusted_host_fails_loud() {
        assert!(assert_trusted_authority("harness.internal/path").is_err());
        assert!(assert_trusted_authority("user@harness.internal").is_err());
    }

    #[test]
    fn privileged_set_matches_typescript() {
        assert!(is_privileged_method("settings.describe"));
        assert!(is_privileged_method("credentials.set"));
        assert!(!is_privileged_method("session.list"));
        assert!(!is_privileged_method("llm.providers"));
        assert!(!is_privileged_method("agentPreset.list"));
    }

    #[test]
    fn rewritten_authorities_fail_loud() {
        assert!(assert_trusted_authority("0x7f.0.0.1").is_err());
        assert!(assert_trusted_authority("harness.internal:").is_err());
        assert!(assert_trusted_authority("[::1]:").is_err());
        assert!(assert_trusted_authority("harness.internal:0080").is_err());
        assert!(assert_trusted_authority("[0:0:0:0:0:0:0:1]").is_err());
        assert!(assert_trusted_authority("harness.internal:3080 ").is_err());
        assert!(assert_trusted_authority("[::1]:3080").is_ok());
        assert!(assert_trusted_authority("HARNESS.internal:80").is_ok());
    }

    #[test]
    fn privileged_requires_loopback_host_header() {
        assert!(privileged_requires_loopback(Some("127.0.0.1:3080")));
        assert!(privileged_requires_loopback(Some("LOCALHOST:3080")));
        assert!(privileged_requires_loopback(Some("[::1]")));
        assert!(!privileged_requires_loopback(None));
        assert!(!privileged_requires_loopback(Some("harness.internal")));
        assert!(!privileged_requires_loopback(Some("bad host")));
    }

    #[test]
    fn origin_host_must_equal_host_host() {
        assert!(is_trusted_api_request(
            Some("127.0.0.1:3080"),
            Some("http://127.0.0.1:3080"),
            None,
            &[],
        ));
        assert!(!is_trusted_api_request(
            Some("127.0.0.1:3080"),
            Some("http://evil.example"),
            None,
            &[],
        ));
        assert!(is_trusted_api_request(
            Some("LOCALHOST:3080"),
            Some("http://localhost:3080"),
            Some("same-origin"),
            &[],
        ));
    }
}
