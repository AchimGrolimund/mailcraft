//! Pictures from the web, for a message whose owner asked to see its pictures.
//!
//! A message names these addresses, so whoever wrote the message chose them. Asking for one must
//! not become a way for a stranger to reach what only the owner can reach, nor tell the stranger
//! more than "somebody asked". [`fetch`] therefore
//!
//! - speaks only to `http://` and `https://` addresses, and refuses one with a user name in it;
//! - sends nothing that says who is asking: no cookie (none is ever kept), no credentials, no
//!   referrer, and the same plain user agent as everything else in this crate;
//! - looks the host's name up itself and refuses the lot when any of its addresses is this
//!   machine, a private or link-local network, or anything else that is not the public internet
//!   ([`is_public`]); it then connects to the very addresses it checked, so a name can't answer
//!   one thing when it is checked and another when it is used;
//! - follows at most [`MAX_REDIRECTS`] redirects, each new address checked in the same way;
//! - takes at most [`MAX_BYTES`] for a picture, the answers of the redirects on the way counted
//!   in, gives the whole of it [`PATIENCE`], and never tries anything twice.
//!
//! Unlike everything else in this crate, a picture may be asked for over plain `http://` from
//! anywhere: nothing secret travels with the request. That is why the connections are made here
//! and not by [`crate::transport`], which goes on refusing unencrypted connections to anything
//! but this machine. The answer is read by [`crate::http`], within its limits.
//!
//! What comes back is as hostile as the message was. Whether it is a picture at all is for
//! whoever decodes it to find out.

use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use mailcraft_proto::ProtoError;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::NetError;
use crate::http::{self, Url};

/// Largest picture taken, in bytes; what the redirects on the way to it answered counts too.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Most redirects followed on the way to one picture.
pub const MAX_REDIRECTS: usize = 3;
/// How long one picture may take, from looking its host up to its last byte, redirects included.
pub const PATIENCE: Duration = Duration::from_secs(20);
/// How long connecting to one address may take.
const CONNECT: Duration = Duration::from_secs(8);
/// How long one read or write may take.
const IO: Duration = Duration::from_secs(10);
/// Most addresses of one host that are tried, and most that a lookup may give.
const MAX_ADDRESSES: usize = 4;
const MAX_LOOKED_UP: usize = 16;
/// What every request says of its sender: nothing about this machine or its owner.
const USER_AGENT: &str = "MailCraft";
const ACCEPT: &str = "image/png, image/jpeg, image/gif, image/*;q=0.5";
/// How much of an answer's beginning is kept, to read where a redirect leads.
const HEAD_KEPT: usize = 20 * 1024;
/// Writes interrupted in a row before giving up.
const MAX_INTERRUPTS: u32 = 100;

/// Why a picture was not downloaded.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PictureError {
    /// Not an `http://` or `https://` address, not an address at all, one with a user name in
    /// it, or one that is too long.
    #[error("the picture's address isn't one that pictures are downloaded from")]
    Address,
    /// The address is this computer's, or of a network that is not the public internet.
    #[error("the picture's address is on this computer or a private network, where mail may not reach")]
    NotPublic,
    #[error("the picture's address led on to too many others")]
    TooManyRedirects,
    #[error("the server answered {0} instead of a picture")]
    Status(u16),
    #[error(transparent)]
    Net(#[from] NetError),
}

/// Download the picture at `url`, an address a message gave, as the module's notes describe.
///
/// It blocks, for [`PATIENCE`] at most, so a host calls it on a background thread. The bytes are
/// whatever the server sent, [`MAX_BYTES`] of them at most: hostile, and not necessarily a picture.
pub fn fetch(url: &str) -> Result<Vec<u8>, PictureError> {
    fetch_with(url, &Reach::web(), &Limits::default())
}

/// How much one picture may cost.
#[derive(Clone, Copy, Debug)]
struct Limits {
    max_bytes: usize,
    patience: Duration,
    connect: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_bytes: MAX_BYTES, patience: PATIENCE, connect: CONNECT }
    }
}

/// Where names lead, and where connections may go.
///
/// There is one of these outside the tests, [`Reach::web`]: the system's names, and the public
/// internet only. The tests have no internet and must not use it, so they alone can name
/// addresses of this machine that stand in for it (`stand_ins` does not exist in any other
/// build, and nothing at run time, no setting and no variable, can add to it).
struct Reach<'a> {
    /// The addresses of a host by its name, found in the time given; none when it has none.
    lookup: &'a dyn Fn(&str, u16, Duration) -> Vec<SocketAddr>,
    #[cfg(test)]
    stand_ins: &'a [SocketAddr],
}

impl Reach<'static> {
    fn web() -> Reach<'static> {
        Reach {
            lookup: &system_lookup,
            #[cfg(test)]
            stand_ins: &[],
        }
    }
}

impl Reach<'_> {
    fn allows(&self, address: &SocketAddr) -> bool {
        #[cfg(test)]
        if self.stand_ins.contains(address) {
            return true;
        }
        is_public(address.ip())
    }
}

/// Whether an address is one of the public internet's: not this machine, not a private,
/// carrier, link-local or site-local network, not multicast or broadcast, and not one of the
/// ranges set aside for documentation, benchmarks, relays and protocols. An IPv6 address that
/// carries an IPv4 one (mapped, or through NAT64) is judged by the one it carries.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => match carried_v4(ip) {
            Some(inner) => is_public_v4(inner),
            None => is_public_v6(ip),
        },
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    let set_aside = a == 0 // "this network"; 0.0.0.0 is this machine on some systems
        || a == 10 // private
        || (a == 100 && (64..128).contains(&b)) // shared by a carrier's customers
        || a == 127 // this machine
        || (a == 169 && b == 254) // link-local, and where clouds keep their metadata
        || (a == 172 && (16..32).contains(&b)) // private
        || (a == 192 && b == 0 && (c == 0 || c == 2)) // protocol assignments; documentation
        || (a == 192 && b == 88 && c == 99) // relays to IPv6
        || (a == 192 && b == 168) // private
        || (a == 198 && (18..20).contains(&b)) // benchmarks
        || (a == 198 && b == 51 && c == 100) // documentation
        || (a == 203 && b == 0 && c == 113) // documentation
        || a >= 224; // multicast, reserved, broadcast
    !set_aside
}

/// The IPv4 address an IPv6 one stands for: `::ffff:a.b.c.d`, or `64:ff9b::a.b.c.d` (NAT64).
fn carried_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return Some(mapped);
    }
    let [a, b, c, d, e, f, g, h] = ip.segments();
    let [g1, g2] = g.to_be_bytes();
    let [h1, h2] = h.to_be_bytes();
    ([a, b, c, d, e, f] == [0x64, 0xff9b, 0, 0, 0, 0]).then(|| Ipv4Addr::new(g1, g2, h1, h2))
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let [a, b, ..] = ip.segments();
    // What is handed out to the internet at large is 2000::/3 (so not ::1, fc00::/7, fe80::/10,
    // ff00::/8), less what is set aside inside it.
    let handed_out = a & 0xe000 == 0x2000;
    let set_aside = (a == 0x2001 && b < 0x0200) // Teredo, benchmarks and other protocol assignments
        || (a == 0x2001 && b == 0x0db8) // documentation
        || a == 0x2002 // 6to4: an IPv4 address in disguise
        || (a == 0x3fff && b < 0x1000); // documentation
    handed_out && !set_aside
}

/// Run `work` on a thread of its own and wait `patience` for it; `None` when it took longer
/// (the thread is left to finish by itself).
fn within<T: Send + 'static>(patience: Duration, work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("picture lookup".into()).spawn(move || {
        let _ = tx.send(work());
    });
    spawned.ok()?;
    rx.recv_timeout(patience).ok()
}

/// The system's answer to "where is `host`". The system takes its own time over a name whose
/// servers don't answer, so it is asked on a thread that can be left behind.
fn system_lookup(host: &str, port: u16, patience: Duration) -> Vec<SocketAddr> {
    let host = host.to_string();
    let look = move || (host.as_str(), port).to_socket_addrs().map(|found| found.take(MAX_LOOKED_UP).collect::<Vec<_>>()).unwrap_or_default();
    within(patience, look).unwrap_or_default()
}

fn problem(text: &str) -> PictureError {
    PictureError::Net(NetError::Proto(ProtoError::Io(text.into())))
}

fn too_slow() -> PictureError {
    problem("the server took too long to answer")
}

fn unreachable_host(host: &str, reason: &str) -> PictureError {
    PictureError::Net(NetError::Connect { host: host.to_string(), reason: reason.to_string() })
}

/// Read `text` as the address of a picture: `http://` or `https://`, a host, a port perhaps, a
/// path. What follows a `#` is dropped (it is never sent).
fn address(text: &str) -> Result<Url, PictureError> {
    let text = text.trim().split('#').next().unwrap_or("");
    let (scheme, rest) = text.split_once("://").ok_or(PictureError::Address)?;
    let secure = match scheme {
        s if s.eq_ignore_ascii_case("https") => true,
        s if s.eq_ignore_ascii_case("http") => false,
        _ => return Err(PictureError::Address),
    };
    // `Url` reads an `https://` address to anywhere, and is as strict with it as is wanted of
    // both kinds: no user name, no white space or control characters, a real port.
    let mut url = Url::parse_within(&format!("https://{rest}"), http::MAX_LONG_URL).map_err(|_| PictureError::Address)?;
    url.secure = secure;
    Ok(url)
}

/// The host as a request names it: with its port when that isn't the usual one.
fn authority(url: &Url) -> String {
    let host = if url.host.contains(':') { format!("[{}]", url.host) } else { url.host.clone() };
    if url.port == 0 { host } else { format!("{host}:{}", url.port) }
}

/// The address a redirect leads to: a whole one, or one written from where `from` is.
fn follow(from: &Url, location: &str) -> Result<Url, PictureError> {
    let location = location.trim();
    if location.is_empty() || location.len() > http::MAX_LONG_URL {
        return Err(PictureError::Address);
    }
    let scheme = if from.secure { "https" } else { "http" };
    let names_scheme = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
    if location.split_once("://").is_some_and(|(before, _)| names_scheme(before)) {
        return address(location);
    }
    if location.starts_with("//") {
        return address(&format!("{scheme}:{location}"));
    }
    if location.starts_with('/') {
        return address(&format!("{scheme}://{}{location}", authority(from)));
    }
    // Beside where `from` is. A first step with a colon in it would be a scheme, not a path.
    let first = location.split(['/', '?']).next().unwrap_or("");
    if first.contains(':') {
        return Err(PictureError::Address);
    }
    let path = from.path.split('?').next().unwrap_or("/");
    let folder = path.rsplit_once('/').map_or("", |(folder, _)| folder);
    address(&format!("{scheme}://{}{folder}/{location}", authority(from)))
}

/// Where a redirect says it leads: the `Location` header of the answer that `head` begins with.
fn location_of(head: &[u8]) -> Option<String> {
    let mut lines = head.split(|b| *b == b'\n').map(|line| line.strip_suffix(b"\r").unwrap_or(line));
    // "Go on" answers may come before the one that counts; `http` puts up with as many.
    for _ in 0..5 {
        let status = String::from_utf8_lossy(lines.next()?).split(' ').filter(|word| !word.is_empty()).nth(1).and_then(|s| s.parse::<u16>().ok())?;
        let mut location = None;
        for line in lines.by_ref() {
            if line.is_empty() {
                break;
            }
            let line = String::from_utf8_lossy(line);
            if let Some((name, value)) = line.split_once(':')
                && name.trim().eq_ignore_ascii_case("location")
                && location.is_none()
            {
                location = Some(value.trim().to_string());
            }
        }
        if !(100..200).contains(&status) {
            return location;
        }
    }
    None
}

/// The addresses of `host` to connect to, every one of them checked.
fn addresses(host: &str, port: u16, reach: &Reach<'_>, deadline: Instant) -> Result<Vec<SocketAddr>, PictureError> {
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    if name == "localhost" || name.ends_with(".localhost") {
        return Err(PictureError::NotPublic);
    }
    let found = match host.parse::<IpAddr>() {
        // An address needs no looking up, and no resolver gets to read it its own way.
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => (reach.lookup)(host, port, deadline.saturating_duration_since(Instant::now())),
    };
    if found.is_empty() {
        return Err(unreachable_host(host, "the name couldn't be looked up"));
    }
    // One address that isn't public refuses them all: a name that points both at the internet
    // and into the owner's network is up to no good.
    if !found.iter().all(|address| reach.allows(address)) {
        return Err(PictureError::NotPublic);
    }
    Ok(found.into_iter().take(MAX_ADDRESSES).collect())
}

/// A connection to the first of `addresses` that takes one, and to nothing else.
fn connect(addresses: &[SocketAddr], host: &str, limits: &Limits, deadline: Instant) -> Result<TcpStream, PictureError> {
    let mut reason = "no address was found for the name";
    for address in addresses {
        let left = deadline.saturating_duration_since(Instant::now()).min(limits.connect);
        if left.is_zero() {
            reason = "the server didn't answer";
            break;
        }
        match TcpStream::connect_timeout(address, left) {
            Ok(tcp) => {
                let _ = tcp.set_nodelay(true);
                return Ok(tcp);
            }
            Err(e) => {
                reason = match e.kind() {
                    ErrorKind::ConnectionRefused => "the server refused the connection",
                    ErrorKind::TimedOut | ErrorKind::WouldBlock => "the server didn't answer",
                    _ => "the network couldn't get there",
                }
            }
        }
    }
    Err(unreachable_host(host, reason))
}

/// A socket that gives up when the picture's time is up: no read or write waits past it.
struct Timed {
    tcp: TcpStream,
    deadline: Instant,
}

impl Timed {
    fn ready(&self) -> std::io::Result<()> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ErrorKind::TimedOut.into());
        }
        // No time at all would mean "wait for ever".
        let wait = Some(left.min(IO).max(Duration::from_millis(1)));
        self.tcp.set_read_timeout(wait)?;
        self.tcp.set_write_timeout(wait)
    }
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.ready()?;
        self.tcp.read(buf)
    }
}

impl Write for Timed {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.ready()?;
        self.tcp.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.ready()?;
        self.tcp.flush()
    }
}

/// A connection, encrypted or not.
enum Line {
    Plain(Timed),
    Tls(Box<StreamOwned<ClientConnection, Timed>>),
}

impl Read for Line {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Line::Plain(plain) => plain.read(buf),
            Line::Tls(tls) => tls.read(buf),
        }
    }
}

impl Write for Line {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Line::Plain(plain) => plain.write(buf),
            Line::Tls(tls) => tls.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Line::Plain(plain) => plain.flush(),
            Line::Tls(tls) => tls.flush(),
        }
    }
}

/// The TLS settings, the same as for mail: certificates must chain to one of Mozilla's roots
/// and match the server's name.
fn tls_config() -> Result<Arc<ClientConfig>, String> {
    static CONFIG: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let config = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|e| e.to_string())?
                .with_root_certificates(roots)
                .with_no_client_auth();
            Ok(Arc::new(config))
        })
        .clone()
}

/// Shake hands over `line` and check the server's certificate against `host`.
fn secure(mut line: Timed, host: &str) -> Result<Line, PictureError> {
    let failed = |reason: String| PictureError::Net(NetError::Tls { host: host.to_string(), reason });
    let config = tls_config().map_err(failed)?;
    let name = ServerName::try_from(host.to_string()).map_err(|_| failed("its name isn't one a certificate can be checked against".into()))?;
    let mut connection = ClientConnection::new(config, name).map_err(|e| failed(e.to_string()))?;
    // Now, so that a bad certificate is reported as one. Every wait in it ends with the picture's time.
    while connection.is_handshaking() {
        connection.complete_io(&mut line).map_err(|e| {
            failed(match e.kind() {
                ErrorKind::TimedOut | ErrorKind::WouldBlock => "the server took too long".into(),
                ErrorKind::UnexpectedEof | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted => "the server closed the connection".into(),
                _ => e.to_string().chars().filter(|c| !c.is_control()).take(200).collect(),
            })
        })?;
    }
    Ok(Line::Tls(Box::new(StreamOwned::new(connection, line))))
}

/// Not `write_all`: that retries an interrupted write for ever.
fn send(line: &mut Line, mut bytes: &[u8]) -> Result<(), PictureError> {
    let failed = |e: &std::io::Error| match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => too_slow(),
        _ => problem("the server closed the connection"),
    };
    let mut interrupts = 0;
    while !bytes.is_empty() {
        match line.write(bytes) {
            Ok(0) => return Err(problem("the server closed the connection")),
            Ok(n) => bytes = bytes.get(n..).unwrap_or(&[]),
            Err(e) if e.kind() == ErrorKind::Interrupted && interrupts < MAX_INTERRUPTS => interrupts += 1,
            Err(e) => return Err(failed(&e)),
        }
    }
    line.flush().map_err(|e| failed(&e))
}

/// A stream that keeps the beginning of what is read from it.
struct Kept<'a, R> {
    inner: &'a mut R,
    head: Vec<u8>,
}

impl<R: Read> Read for Kept<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        let room = HEAD_KEPT.saturating_sub(self.head.len());
        self.head.extend_from_slice(buf.get(..n.min(room)).unwrap_or(&[]));
        Ok(n)
    }
}

/// What one address answered.
struct Answer {
    status: u16,
    body: Vec<u8>,
    /// Where it sends the asker on to, when it is a redirect that says.
    location: Option<String>,
}

/// One GET, on a connection of its own, to an address that was checked first.
fn ask(url: &Url, reach: &Reach<'_>, limits: &Limits, max_bytes: usize, deadline: Instant) -> Result<Answer, PictureError> {
    let port = match (url.port, url.secure) {
        (0, true) => 443,
        (0, false) => 80,
        (port, _) => port,
    };
    let addresses = addresses(&url.host, port, reach, deadline)?;
    let tcp = connect(&addresses, &url.host, limits, deadline)?;
    let line = Timed { tcp, deadline };
    let mut line = if url.secure { secure(line, &url.host)? } else { Line::Plain(line) };
    // The address was checked when it was read: nothing in it can end a line early. And this is
    // all that is ever said: what to send, to whom, and that nothing more will be asked.
    let request =
        format!("GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {USER_AGENT}\r\nAccept: {ACCEPT}\r\nConnection: close\r\n\r\n", url.path, authority(url));
    send(&mut line, request.as_bytes())?;
    let mut kept = Kept { inner: &mut line, head: Vec::new() };
    let started = Instant::now();
    let reply = http::read_reply(&mut kept, started, deadline.saturating_duration_since(started), max_bytes)?;
    let location = if (300..400).contains(&reply.status) { location_of(&kept.head) } else { None };
    log::debug!("a picture's server answered {} ({} bytes)", reply.status, reply.body.len());
    Ok(Answer { status: reply.status, body: reply.body, location })
}

/// [`fetch`], with the reach and the limits said.
fn fetch_with(url: &str, reach: &Reach<'_>, limits: &Limits) -> Result<Vec<u8>, PictureError> {
    let deadline = Instant::now() + limits.patience;
    let mut at = address(url)?;
    let mut left = limits.max_bytes;
    // The address itself, and then as many redirects as are followed.
    for _ in 0..=MAX_REDIRECTS {
        let answer = ask(&at, reach, limits, left, deadline)?;
        match answer.status {
            200 => return Ok(answer.body),
            301 | 302 | 303 | 307 | 308 => {
                let location = answer.location.ok_or(PictureError::Status(answer.status))?;
                at = follow(&at, &location)?;
                left = left.saturating_sub(answer.body.len());
            }
            status => return Err(PictureError::Status(status)),
        }
    }
    Err(PictureError::TooManyRedirects)
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use proptest::prelude::*;

    use super::*;
    use crate::fake::FakeHttp;

    /// Something a server could send as a picture (whether it is one is not this crate's business).
    fn picture() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend((0..600u32).map(|i| (i % 251) as u8));
        bytes
    }

    fn answer(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn redirect(to: &str) -> Vec<u8> {
        answer("302 Found", &format!("Location: {to}\r\n"), b"moved")
    }

    /// The fake's own address, where the tests allow connections to go.
    fn at(server: &FakeHttp) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], server.port))
    }

    /// The path a request asks for.
    fn path_of(request: &str) -> &str {
        request.split(' ').nth(1).unwrap_or("")
    }

    /// Limits for tests: the same sizes, less waiting.
    fn quick() -> Limits {
        Limits { patience: Duration::from_secs(5), connect: Duration::from_secs(3), ..Limits::default() }
    }

    /// For addresses that are refused by how they are written: no name is ever looked up.
    fn no_names(host: &str, _: u16, _: Duration) -> Vec<SocketAddr> {
        panic!("{host} was looked up")
    }

    fn protocol(result: Result<Vec<u8>, PictureError>) -> String {
        match result {
            Err(PictureError::Net(NetError::Proto(ProtoError::Protocol(why)))) => why,
            other => panic!("expected an answer that makes no sense, got {:?}", other.map(|bytes| bytes.len())),
        }
    }

    #[test]
    fn a_picture_is_fetched_and_the_request_says_nothing_of_its_sender() {
        let server = FakeHttp::answering(answer("200 OK", "Content-Type: image/png\r\nSet-Cookie: who=you; Path=/\r\n", &picture()));
        let allowed = [at(&server)];
        let names = |host: &str, _: u16, _: Duration| if host.trim_end_matches('.') == "pictures.example" { allowed.to_vec() } else { Vec::new() };
        let reach = Reach { lookup: &names, stand_ins: &allowed };

        // By its address, and by a name that leads there; plain `http` to a name is fine for a picture.
        assert_eq!(fetch_with(&server.url("/img/hero.png?w=600&sig=abc#top"), &reach, &quick()), Ok(picture()));
        assert_eq!(fetch_with(" HTTP://pictures.example/logo.png ", &reach, &quick()), Ok(picture()));
        assert_eq!(fetch_with("http://pictures.example./logo.png", &reach, &quick()), Ok(picture()));

        let sent = server.request(0);
        assert!(sent.starts_with("GET /img/hero.png?w=600&sig=abc HTTP/1.1\r\n"), "{sent}");
        assert!(sent.ends_with("\r\n\r\n"), "a GET has no body: {sent}");
        // Exactly these headers, and so no cookie, no credentials, no referrer, nothing of this machine.
        let headers: Vec<(String, String)> =
            sent.lines().skip(1).filter_map(|l| l.split_once(": ")).map(|(name, value)| (name.to_ascii_lowercase(), value.to_string())).collect();
        assert_eq!(
            headers,
            [
                ("host", format!("127.0.0.1:{}", server.port).as_str()),
                ("user-agent", "MailCraft"),
                ("accept", "image/png, image/jpeg, image/gif, image/*;q=0.5"),
                ("connection", "close"),
            ]
            .map(|(name, value)| (name.to_string(), value.to_string()))
        );
        // The cookie the first answer set is not sent with the next request, to the same server.
        for n in 0..3 {
            let lower = server.request(n).to_ascii_lowercase();
            for never in ["cookie", "authorization", "referer", "origin", "from:", "who=you"] {
                assert!(!lower.contains(never), "{never}: {lower}");
            }
        }
        assert!(server.request(1).contains("\r\nHost: pictures.example\r\n"));
        assert_eq!(server.asked(), 3);
    }

    #[test]
    fn redirects_are_followed_three_times_and_no_further() {
        let server = FakeHttp::start(|_, request| {
            let host = request.lines().find_map(|l| l.strip_prefix("Host: ")).unwrap_or("").to_string();
            match path_of(request) {
                // Written every way a server writes them.
                "/one" => answer("301 Moved Permanently", "Set-Cookie: seen=1\r\nlocation:  /two?x=1  \r\n", b""),
                "/two?x=1" => answer("307 Temporary Redirect", &format!("Location: http://{host}/dir/three\r\n"), b""),
                "/dir/three" => redirect("four.png#frag"),
                "/dir/four.png" => answer("200 OK", "", &picture()),
                "/same-host" => answer("303 See Other", &format!("Location: //{host}/dir/four.png\r\n"), b""),
                "/four-hops" => redirect("/one"),
                "/loop" => redirect("/loop"),
                "/nowhere" => answer("302 Found", "", b""),
                "/empty" => answer("302 Found", "Location: \r\n", b""),
                "/not-modified" => answer("304 Not Modified", "Location: /dir/four.png\r\n", b""),
                "/gone" => answer("404 Not Found", "", b"no such picture"),
                _ => answer("500 Oops", "", b""),
            }
        });
        let allowed = [at(&server)];
        let reach = Reach { lookup: &no_names, stand_ins: &allowed };
        let get = |path: &str| fetch_with(&server.url(path), &reach, &quick());

        assert_eq!(get("/one"), Ok(picture()));
        assert_eq!(server.asked(), 4);
        // What the first answer set as a cookie went nowhere.
        assert!((0..4).all(|n| !server.request(n).to_ascii_lowercase().contains("cookie")));
        assert_eq!(get("/same-host"), Ok(picture()));
        // A fourth redirect is one too many, and a loop ends the same way: after four requests.
        let before = server.asked();
        assert_eq!(get("/four-hops"), Err(PictureError::TooManyRedirects));
        assert_eq!(server.asked() - before, 4);
        let before = server.asked();
        assert_eq!(get("/loop"), Err(PictureError::TooManyRedirects));
        assert_eq!(server.asked() - before, 4);
        // A redirect that doesn't say where, and answers that are neither a picture nor a redirect.
        assert_eq!(get("/nowhere"), Err(PictureError::Status(302)));
        assert_eq!(get("/empty"), Err(PictureError::Address));
        assert_eq!(get("/not-modified"), Err(PictureError::Status(304)));
        assert_eq!(get("/gone"), Err(PictureError::Status(404)));
        assert_eq!(get("/anything-else"), Err(PictureError::Status(500)));
    }

    #[test]
    fn a_redirect_is_checked_like_the_address_it_came_from() {
        // A second server on this machine that the tests were not told about: as far as the
        // check goes it is the owner's own machine, and must never hear from us.
        let inside = FakeHttp::answering(answer("200 OK", "", b"the owner's router"));
        let inside_port = inside.port;
        let server = FakeHttp::start(move |_, request| {
            let me = request.lines().find_map(|l| l.strip_prefix("Host: ")).and_then(|h| h.rsplit(':').next()).unwrap_or("").to_string();
            let to = match path_of(request) {
                "/0" => format!("http://127.0.0.1:{inside_port}/admin"),
                // This server's own port, by names for this machine rather than its address.
                "/1" => format!("http://localhost:{me}/x.png"),
                "/2" => format!("http://[::1]:{me}/x.png"),
                "/3" => format!("http://[::ffff:127.0.0.1]:{me}/x.png"),
                "/4" => "http://10.0.0.1/x.png".into(),
                "/5" => "https://192.168.1.1/x.png".into(),
                "/6" => "http://169.254.169.254/latest/meta-data/".into(),
                "/7" => "http://[fd00::1]/x.png".into(),
                "/8" => "http://0.0.0.0/x.png".into(),
                "/9" => "https://intranet.example/x.png".into(),
                "/10" => "http://both.example/x.png".into(),
                "/11" => "ftp://pictures.example/x.png".into(),
                "/12" => "file:///etc/passwd".into(),
                "/13" => "javascript:alert(1)".into(),
                "/14" => "data:image/png;base64,AAAA".into(),
                "/15" => "//owner:secret@pictures.example/x.png".into(),
                "/16" => "http://pictures.example:0/x.png".into(),
                _ => "/missing".into(),
            };
            redirect(&to)
        });
        let allowed = [at(&server)];
        let looked_up = Mutex::new(Vec::new());
        let names = |host: &str, port: u16, _: Duration| {
            looked_up.lock().unwrap().push(host.to_string());
            match host {
                "intranet.example" => vec![SocketAddr::from(([10, 1, 2, 3], port))],
                // An address of the internet and one of this machine: refused whole, and neither is tried.
                "both.example" => vec![SocketAddr::from(([93, 184, 216, 34], port)), SocketAddr::from(([127, 0, 0, 1], inside_port))],
                _ => Vec::new(),
            }
        };
        let reach = Reach { lookup: &names, stand_ins: &allowed };
        for n in 0..=16 {
            let wanted = if n <= 10 { PictureError::NotPublic } else { PictureError::Address };
            assert_eq!(fetch_with(&server.url(&format!("/{n}")), &reach, &quick()), Err(wanted), "/{n}");
        }
        // Each of them cost one request, to the server that sent the redirect, and nothing else.
        assert_eq!(server.asked(), 17);
        assert_eq!(inside.asked(), 0);
        assert_eq!(*looked_up.lock().unwrap(), ["intranet.example", "both.example"]);
    }

    #[test]
    fn addresses_that_are_not_public_are_refused_before_anything_is_sent() {
        // With the reach every other build has, a server on this machine is out of reach, though
        // it is there and would answer. (Only addresses here: the tests look no real name up.)
        let server = FakeHttp::answering(answer("200 OK", "", &picture()));
        let port = server.port;
        for local in [
            format!("http://127.0.0.1:{port}/a.png"),
            format!("http://localhost:{port}/a.png"),
            format!("http://LOCALHOST.:{port}/a.png"),
            format!("http://mail.localhost:{port}/a.png"),
            format!("http://127.8.9.10:{port}/a.png"),
            format!("http://[::1]:{port}/a.png"),
            format!("http://[::ffff:127.0.0.1]:{port}/a.png"),
            format!("http://[::ffff:7f00:1]:{port}/a.png"),
            format!("http://0.0.0.0:{port}/a.png"),
            format!("https://127.0.0.1:{port}/a.png"),
        ] {
            assert_eq!(fetch(&local), Err(PictureError::NotPublic), "{local}");
        }
        assert_eq!(server.asked(), 0);

        // Nor is a name looked up for an address that is refused by how it is written.
        let reach = Reach { lookup: &no_names, stand_ins: &[] };
        for (url, wanted) in [
            ("http://10.0.0.1/a.png", PictureError::NotPublic),
            ("http://10.255.255.255:8080/a.png", PictureError::NotPublic),
            ("http://172.16.0.1/a.png", PictureError::NotPublic),
            ("http://172.31.255.254/a.png", PictureError::NotPublic),
            ("http://192.168.0.1/a.png", PictureError::NotPublic),
            ("http://100.64.0.1/a.png", PictureError::NotPublic),
            ("http://169.254.169.254/latest/meta-data/iam/", PictureError::NotPublic),
            ("http://224.0.0.251/a.png", PictureError::NotPublic),
            ("http://255.255.255.255/a.png", PictureError::NotPublic),
            ("http://[fe80::1]/a.png", PictureError::NotPublic),
            ("http://[fc00::1]/a.png", PictureError::NotPublic),
            ("http://[fd12:3456:789a::1]/a.png", PictureError::NotPublic),
            ("http://[::]/a.png", PictureError::NotPublic),
            ("http://[64:ff9b::a00:1]/a.png", PictureError::NotPublic),
            ("http://[2001:db8::1]/a.png", PictureError::NotPublic),
            ("http://[2002:c0a8:101::1]/a.png", PictureError::NotPublic),
            ("https://localhost/a.png", PictureError::NotPublic),
            ("", PictureError::Address),
            ("pictures.example/a.png", PictureError::Address),
            ("//pictures.example/a.png", PictureError::Address),
            ("/a.png", PictureError::Address),
            ("ftp://pictures.example/a.png", PictureError::Address),
            ("file:///C:/Users/owner/secret.png", PictureError::Address),
            ("\\\\fileserver\\share\\a.png", PictureError::Address),
            ("cid:logo@example.org", PictureError::Address),
            ("data:image/png;base64,AAAA", PictureError::Address),
            ("javascript:alert(1)", PictureError::Address),
            ("https://owner:secret@pictures.example/a.png", PictureError::Address),
            ("https://owner@pictures.example/a.png", PictureError::Address),
            ("https://pictures.example:0/a.png", PictureError::Address),
            ("https://pictures.example:99999/a.png", PictureError::Address),
            ("https://pictures.example/a b.png", PictureError::Address),
            ("https://pictures.example/a.png\r\nX-Injected: 1", PictureError::Address),
            ("https://pictüres.example/a.png", PictureError::Address),
            ("https://fe80::1%eth0/a.png", PictureError::Address),
            ("https://[fe80::1%25eth0]/a.png", PictureError::Address),
            ("https://", PictureError::Address),
            ("https:///a.png", PictureError::Address),
        ] {
            assert_eq!(fetch_with(url, &reach, &quick()), Err(wanted), "{url:?}");
        }
        let long = format!("https://pictures.example/{}", "a".repeat(http::MAX_LONG_URL));
        assert_eq!(fetch_with(&long, &reach, &quick()), Err(PictureError::Address));

        // By name: every address a name has must be public; a name with none is no host.
        let names = |host: &str, port: u16, _: Duration| match host {
            "router.example" => vec![SocketAddr::from(([192, 168, 1, 1], port))],
            "sly.example" => vec![SocketAddr::from(([93, 184, 216, 34], port)), SocketAddr::from(([10, 0, 0, 7], port))],
            "six.example" => vec![SocketAddr::new("fe80::1".parse().unwrap(), port)],
            _ => Vec::new(),
        };
        let reach = Reach { lookup: &names, stand_ins: &[] };
        for host in ["router.example", "sly.example", "six.example"] {
            assert_eq!(fetch_with(&format!("https://{host}/a.png"), &reach, &quick()), Err(PictureError::NotPublic), "{host}");
        }
        assert!(matches!(
            fetch_with("https://nowhere.example/a.png", &reach, &quick()),
            Err(PictureError::Net(NetError::Connect { host, .. })) if host == "nowhere.example"
        ));
    }

    #[test]
    fn what_is_public_and_what_is_not() {
        let ip = |text: &str| text.parse::<IpAddr>().unwrap();
        for public in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "100.63.255.255",
            "100.128.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "192.0.1.1",
            "192.167.255.255",
            "198.17.255.255",
            "198.20.0.0",
            "223.255.255.255",
            "2001:4860:4860::8888",
            "2606:4700:4700::1111",
            "2a00:1450:4001:81b::200e",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
        ] {
            assert!(is_public(ip(public)), "{public}");
        }
        for private in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.0",
            "10.255.255.255",
            "100.64.0.0",
            "100.127.255.255",
            "127.0.0.1",
            "127.255.255.255",
            "169.254.0.1",
            "169.254.169.254",
            "172.16.0.0",
            "172.31.255.255",
            "192.0.0.1",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.0.0",
            "192.168.255.255",
            "198.18.0.0",
            "198.19.255.255",
            "198.51.100.7",
            "203.0.113.7",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::7f00:1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:192.168.1.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a00:1",
            "64:ff9b:1::808:808",
            "100::1",
            "2001::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "2001:2::1",
            "2001:db8::1",
            "2002:808:808::1",
            "3fff::1",
            "fc00::1",
            "fd00::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "4000::1",
        ] {
            assert!(!is_public(ip(private)), "{private}");
        }
    }

    #[test]
    fn a_picture_that_is_too_large_is_refused() {
        let small = Limits { max_bytes: 10_000, ..quick() };
        let server = FakeHttp::start(|_, request| {
            let body = |n: usize| "x".repeat(n);
            match path_of(request) {
                "/fits" => answer("200 OK", "", body(10_000).as_bytes()),
                "/by-length" => answer("200 OK", "", body(10_001).as_bytes()),
                "/in-chunks" => {
                    let chunk = format!("1000\r\n{}\r\n", body(0x1000));
                    format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{chunk}{chunk}{chunk}0\r\n\r\n").into_bytes()
                }
                "/to-the-end" => format!("HTTP/1.1 200 OK\r\n\r\n{}", body(10_001)).into_bytes(),
                // Claimed, and never sent: refused on the claim.
                "/claimed" => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\npng", MAX_BYTES + 1).into_bytes(),
                "/absurd" => b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999999999999999\r\n\r\npng".to_vec(),
                // What the redirects on the way answer counts too.
                "/heavy-1" => answer("302 Found", "Location: /heavy-2\r\n", body(6_000).as_bytes()),
                "/heavy-2" => answer("302 Found", "Location: /heavy-3\r\n", body(3_000).as_bytes()),
                "/heavy-3" => answer("200 OK", "", body(1_001).as_bytes()),
                "/light-1" => answer("302 Found", "Location: /light-2\r\n", body(6_000).as_bytes()),
                "/light-2" => answer("200 OK", "", body(4_000).as_bytes()),
                _ => answer("404 Not Found", "", b""),
            }
        });
        let allowed = [at(&server)];
        let reach = Reach { lookup: &no_names, stand_ins: &allowed };
        let get = |path: &str, limits: &Limits| fetch_with(&server.url(path), &reach, limits);

        assert_eq!(get("/fits", &small).map(|bytes| bytes.len()), Ok(10_000));
        for path in ["/by-length", "/in-chunks", "/to-the-end"] {
            assert!(protocol(get(path, &small)).contains("longer than 9 KB"), "{path}");
        }
        assert_eq!(get("/light-1", &small).map(|bytes| bytes.len()), Ok(4_000));
        assert!(protocol(get("/heavy-1", &small)).contains("longer than"));
        // The size every other build goes by.
        assert_eq!(Limits::default().max_bytes, MAX_BYTES);
        assert!(protocol(get("/claimed", &quick())).contains("longer than 8192 KB"));
        assert!(matches!(get("/absurd", &quick()), Err(PictureError::Net(NetError::Proto(ProtoError::Protocol(_))))));
    }

    /// A server on this machine that does with its one connection what `serve` says.
    fn serving(serve: impl FnOnce(TcpStream) + Send + 'static) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((peer, _)) = listener.accept() {
                serve(peer);
            }
        });
        address
    }

    #[test]
    fn a_slow_server_is_given_up_on() {
        let hurried = Limits { patience: Duration::from_millis(400), ..Limits::default() };
        let slow = |result: Result<Vec<u8>, PictureError>| match result {
            Err(PictureError::Net(NetError::Proto(ProtoError::Io(why)))) => why,
            other => panic!("expected a server that is too slow, got {:?}", other.map(|bytes| bytes.len())),
        };
        // It takes the connection and the request, and never says a word.
        let (hold, held) = std::sync::mpsc::channel::<()>();
        let silent = serving(move |peer| {
            let _ = held.recv();
            drop(peer);
        });
        let started = Instant::now();
        let result = fetch_with(&format!("http://{silent}/a.png"), &Reach { lookup: &no_names, stand_ins: &[silent] }, &hurried);
        assert_eq!(slow(result), "the server took too long to answer");
        assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
        drop(hold);

        // A byte every 20 ms, for ever: no single read waits long, and the picture never ends.
        let dripping = serving(|mut peer| {
            let _ = peer.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
            while peer.write_all(b"x").is_ok() {
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let started = Instant::now();
        let result = fetch_with(&format!("http://{dripping}/a.png"), &Reach { lookup: &no_names, stand_ins: &[dripping] }, &hurried);
        assert_eq!(slow(result), "the server took too long to answer");
        assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());

        // Redirects don't each get their time anew: the picture has one allowance.
        let server = FakeHttp::start(|n, _| {
            std::thread::sleep(Duration::from_millis(250));
            redirect(&format!("/{}", n + 1))
        });
        let allowed = [at(&server)];
        let started = Instant::now();
        let result = fetch_with(&server.url("/0"), &Reach { lookup: &no_names, stand_ins: &allowed }, &hurried);
        assert!(result.is_err() && result != Err(PictureError::TooManyRedirects), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());

        // A name whose lookup hangs is left behind when the time is up.
        let started = Instant::now();
        assert_eq!(within(Duration::from_millis(100), || std::thread::sleep(Duration::from_secs(3))), None);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(within(Duration::from_secs(5), || 7), Some(7));
        let stuck = |_: &str, _: u16, patience: Duration| {
            within(patience, || std::thread::sleep(Duration::from_secs(3))).map(|()| Vec::new()).unwrap_or_default()
        };
        let started = Instant::now();
        let result = fetch_with("https://stuck.example/a.png", &Reach { lookup: &stuck, stand_ins: &[] }, &hurried);
        assert!(matches!(result, Err(PictureError::Net(NetError::Connect { .. }))), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    fn a_closed_port_and_a_server_that_hangs_up_are_errors() {
        // A port that was just free: nothing listens there.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let result = fetch_with(&format!("http://{closed}/a.png"), &Reach { lookup: &no_names, stand_ins: &[closed] }, &quick());
        assert!(matches!(&result, Err(PictureError::Net(NetError::Connect { host, .. })) if host == "127.0.0.1"), "{result:?}");
        // It answers the door and leaves.
        let rude = serving(drop);
        let result = fetch_with(&format!("http://{rude}/a.png"), &Reach { lookup: &no_names, stand_ins: &[rude] }, &quick());
        assert!(matches!(result, Err(PictureError::Net(NetError::Proto(ProtoError::Io(_))))), "{result:?}");
        // `https` to a server that speaks no TLS is a failed handshake, not a picture.
        let plain = serving(|mut peer| {
            let _ = peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\npng");
            let _ = peer.read(&mut [0u8; 512]);
        });
        let result = fetch_with(&format!("https://{plain}/a.png"), &Reach { lookup: &no_names, stand_ins: &[plain] }, &quick());
        assert!(matches!(&result, Err(PictureError::Net(NetError::Tls { host, .. })) if host == "127.0.0.1"), "{result:?}");
    }

    #[test]
    fn hostile_answers_are_errors_or_bytes_and_never_more() {
        let answers: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"\x00\xff\xfe garbage \x80".to_vec(),
            b"220 smtp.example.org ESMTP\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: many\r\n\r\npng".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\ncut short".to_vec(),
            b"HTTP/1.1 302 Found\r\nLocation: \xff\xfe\x00\r\n\r\n".to_vec(),
            b"HTTP/1.1 302 Found\r\nLocation: http://\r\n\r\n".to_vec(),
            format!("HTTP/1.1 302 Found\r\nLocation: /{}\r\n\r\n", "a".repeat(12_000)).into_bytes(),
            format!("HTTP/1.1 200 OK\r\n{}\r\npng", "X-Filler: yes\r\n".repeat(400)).into_bytes(),
            format!("HTTP/1.1 200 OK\r\nX-Filler: {}\r\n\r\npng", "x".repeat(40_000)).into_bytes(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\npng\r\n0\r\n\r\n".to_vec(),
            b"HTTP/1.1 999 What\r\n\r\n".to_vec(),
        ];
        let count = answers.len();
        let server = FakeHttp::start(move |n, _| answers[n % count].clone());
        let allowed = [at(&server)];
        let reach = Reach { lookup: &no_names, stand_ins: &allowed };
        for n in 0..count {
            let result = fetch_with(&server.url("/a.png"), &reach, &quick());
            assert!(result.is_err(), "answer {n}: {:?}", result.map(|bytes| bytes.len()));
        }
        // What is not a picture and says 200 is handed on as it came: telling is the decoder's work.
        let page = FakeHttp::answering(answer("200 OK", "Content-Type: text/html\r\n", b"<html><script>alert(1)</script></html>"));
        let allowed = [at(&page)];
        let bytes = fetch_with(&page.url("/a.png"), &Reach { lookup: &no_names, stand_ins: &allowed }, &quick()).unwrap();
        assert!(bytes.starts_with(b"<html>"));
    }

    #[test]
    fn addresses_are_read_and_redirects_are_resolved() {
        let read = |text: &str| address(text).map(|url| (url.secure, url.host, url.port, url.path));
        assert_eq!(read("https://img.example/a/b.png?w=2"), Ok((true, "img.example".into(), 0, "/a/b.png?w=2".into())));
        assert_eq!(read("http://IMG.example:8080"), Ok((false, "IMG.example".into(), 8080, "/".into())));
        assert_eq!(read("hTTp://[2001:4860::1]:81/x#y"), Ok((false, "2001:4860::1".into(), 81, "/x".into())));
        assert_eq!(read("https://img.example?x=1"), Ok((true, "img.example".into(), 0, "/?x=1".into())));

        let from = address("https://img.example:8443/shop/2026/a.png?size=2").unwrap();
        let to = |location: &str| {
            follow(&from, location).map(|url| format!("{}://{}{}", if url.secure { "https" } else { "http" }, authority(&url), url.path))
        };
        assert_eq!(to("http://cdn.example/b.png").as_deref(), Ok("http://cdn.example/b.png"));
        assert_eq!(to("HTTPS://cdn.example:444/b.png#x").as_deref(), Ok("https://cdn.example:444/b.png"));
        assert_eq!(to("//cdn.example/b.png").as_deref(), Ok("https://cdn.example/b.png"));
        assert_eq!(to("/b.png?u=http://x.example/").as_deref(), Ok("https://img.example:8443/b.png?u=http://x.example/"));
        assert_eq!(to("b.png").as_deref(), Ok("https://img.example:8443/shop/2026/b.png"));
        assert_eq!(to("?size=3").as_deref(), Ok("https://img.example:8443/shop/2026/?size=3"));
        assert_eq!(
            follow(&address("http://[::1]:81").unwrap(), "x.png").map(|url| (authority(&url), url.path)),
            Ok(("[::1]:81".into(), "/x.png".into()))
        );
        for bad in [
            "",
            "  ",
            "ftp://cdn.example/b.png",
            "javascript:alert(1)",
            "mailto:a@b.example",
            "data:,x",
            "b.png\r\nHost: evil",
            "http://a b/",
            "//a@b/c",
        ] {
            assert_eq!(to(bad), Err(PictureError::Address), "{bad:?}");
        }

        let head = |text: &str| location_of(text.as_bytes());
        assert_eq!(head("HTTP/1.1 302 Found\r\nLocation: /b.png\r\n\r\nbody").as_deref(), Some("/b.png"));
        assert_eq!(head("HTTP/1.0 301\nX: 1\nLOCATION:   https://cdn.example/b.png  \n\n").as_deref(), Some("https://cdn.example/b.png"));
        assert_eq!(
            head("HTTP/1.1 100 Continue\r\nLocation: /wrong\r\n\r\nHTTP/1.1 302 Found\r\nLocation: /right\r\n\r\n").as_deref(),
            Some("/right")
        );
        // The first one counts; one in the body is no header.
        assert_eq!(head("HTTP/1.1 302 Found\r\nLocation: /first\r\nLocation: /second\r\n\r\n").as_deref(), Some("/first"));
        assert_eq!(head("HTTP/1.1 302 Found\r\n\r\nLocation: /in-the-body\r\n"), None);
        assert_eq!(head("HTTP/1.1 302 Found\r\nX-Location: /no\r\n\r\n"), None);
        assert_eq!(head(""), None);
        assert_eq!(head("nonsense"), None);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Whatever a server sends, the answer is an error or bytes within the limit.
        #[test]
        fn arbitrary_answers_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400), after_head in any::<bool>()) {
            static SERVER: OnceLock<(FakeHttp, Arc<Mutex<Vec<u8>>>, AtomicUsize)> = OnceLock::new();
            let (server, next, asked) = SERVER.get_or_init(|| {
                let next: Arc<Mutex<Vec<u8>>> = Default::default();
                let from = next.clone();
                (FakeHttp::start(move |_, _| from.lock().unwrap().clone()), next, AtomicUsize::new(0))
            });
            let mut answer = if after_head { b"HTTP/1.1 302 Found\r\nLocation: ".to_vec() } else { Vec::new() };
            answer.extend_from_slice(&bytes);
            *next.lock().unwrap() = answer;
            let allowed = [at(server)];
            let limits = Limits { max_bytes: 200, patience: Duration::from_secs(5), connect: Duration::from_secs(3) };
            // A redirect could name a host: none has an address here.
            let nowhere = |_: &str, _: u16, _: Duration| Vec::new();
            let url = server.url(&format!("/{}", asked.fetch_add(1, Ordering::Relaxed)));
            if let Ok(body) = fetch_with(&url, &Reach { lookup: &nowhere, stand_ins: &allowed }, &limits) {
                prop_assert!(body.len() <= 200);
            }
        }

        #[test]
        fn any_text_is_an_address_or_not(text in "\\PC{0,60}", tail in "[a-z0-9:/?#@\\[\\].%-]{0,40}", location in "\\PC{0,40}|[a-z0-9:/?#@\\[\\].%-]{0,40}") {
            let _ = address(&text);
            let _ = address(&tail);
            let _ = location_of(text.as_bytes());
            let other = format!("ftp://{tail}");
            prop_assert!(address(&other).is_err());
            for scheme in ["https://", "http://", "HTTP://"] {
                if let Ok(url) = address(&format!("{scheme}{tail}")) {
                    prop_assert!(!url.host.is_empty() && !url.host.contains('@') && url.path.starts_with('/') && !url.path.contains('#'));
                    prop_assert_eq!(url.secure, scheme == "https://");
                    // Wherever a redirect points, what comes of it is an address like any other, or nothing.
                    if let Ok(next) = follow(&url, &location) {
                        prop_assert!(!next.host.is_empty() && next.path.starts_with('/') && next.path.bytes().all(|b| b.is_ascii_graphic()));
                    }
                }
            }
        }

        #[test]
        fn every_address_is_public_or_not(v4 in any::<u32>(), v6 in any::<u128>()) {
            let v4 = Ipv4Addr::from(v4);
            // Private, loopback and link-local by the standard library's own reckoning are never public here.
            if v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() || v4.is_multicast() || v4.is_broadcast() || v4.is_documentation() {
                prop_assert!(!is_public(IpAddr::V4(v4)));
            }
            // And an IPv6 address that carries an IPv4 one is public exactly when that one is.
            prop_assert_eq!(is_public(IpAddr::V6(v4.to_ipv6_mapped())), is_public(IpAddr::V4(v4)));
            let v6 = Ipv6Addr::from(v6);
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || v6.is_unique_local() || v6.is_unicast_link_local() {
                prop_assert!(!is_public(IpAddr::V6(v6)));
            }
        }
    }
}
