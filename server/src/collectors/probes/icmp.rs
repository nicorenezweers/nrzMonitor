//! ICMP-Echo (IPv4) ohne externes Programm.
//!
//! Zuerst ein „Ping-Socket“ (`SOCK_DGRAM` + `IPPROTO_ICMP`, unter Linux erlaubt, wenn die Gruppe in
//! `net.ipv4.ping_group_range` liegt; auf macOS immer), sonst ein Raw-Socket (braucht `CAP_NET_RAW`,
//! das Docker standardmässig gibt). Blockiert – aus async-Code über `spawn_blocking`.

use std::{
    net::Ipv4Addr,
    time::{Duration, Instant},
};

/// Ergebnis mehrerer Echos.
#[derive(Debug, Clone, PartialEq)]
pub struct PingResult {
    pub sent: u32,
    pub received: u32,
    /// Mittlere Antwortzeit (ms) der erhaltenen Antworten.
    pub avg_ms: Option<f64>,
}

struct Socket(libc::c_int);

impl Drop for Socket {
    fn drop(&mut self) {
        // SAFETY: eigener, gültiger Dateideskriptor.
        unsafe { libc::close(self.0) };
    }
}

fn open() -> std::io::Result<Socket> {
    // SAFETY: einfache Systemaufrufe ohne Zeiger.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, libc::IPPROTO_ICMP) };
    if fd >= 0 {
        return Ok(Socket(fd));
    }
    // SAFETY: wie oben.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_RAW, libc::IPPROTO_ICMP) };
    if fd >= 0 {
        return Ok(Socket(fd));
    }
    Err(std::io::Error::last_os_error())
}

/// Darf dieser Prozess pingen?
pub fn available() -> Result<(), String> {
    open().map(|_| ()).map_err(|e| format!("ICMP nicht erlaubt ({e}) – Ping prüft ersatzweise per TCP."))
}

fn checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in data.chunks(2) {
        let word = u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]);
        sum += u32::from(word);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn sockaddr(ip: Ipv4Addr) -> libc::sockaddr_in {
    // SAFETY: sockaddr_in ist ein einfaches C-Struct; Nullen sind gültig.
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sa.sin_family = libc::AF_INET as libc::sa_family_t;
    sa.sin_addr = libc::in_addr { s_addr: u32::from_ne_bytes(ip.octets()) };
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    {
        sa.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
    }
    sa
}

/// `count` Echos im Abstand von 200 ms, je höchstens `timeout` warten.
pub fn ping(ip: Ipv4Addr, count: u32, timeout: Duration) -> std::io::Result<PingResult> {
    let sock = open()?;
    let tv = libc::timeval { tv_sec: 0, tv_usec: 100_000 };
    // SAFETY: gültiger Deskriptor und Zeiger auf eine lokale timeval.
    unsafe {
        libc::setsockopt(
            sock.0,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&raw const tv).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }
    let ident = (std::process::id() as u16) ^ fastrand::u16(..);
    let cookie: [u8; 8] = fastrand::u64(..).to_be_bytes();
    let addr = sockaddr(ip);
    let mut times = Vec::new();
    for seq in 0..count as u16 {
        let mut pkt = [0u8; 16];
        pkt[0] = 8; // Echo Request
        pkt[4..6].copy_from_slice(&ident.to_be_bytes());
        pkt[6..8].copy_from_slice(&seq.to_be_bytes());
        pkt[8..16].copy_from_slice(&cookie);
        let sum = checksum(&pkt);
        pkt[2..4].copy_from_slice(&sum.to_be_bytes());
        let sent = Instant::now();
        // SAFETY: Puffer und Adresse sind gültig und leben während des Aufrufs.
        let n = unsafe {
            libc::sendto(
                sock.0,
                pkt.as_ptr().cast(),
                pkt.len(),
                0,
                (&raw const addr).cast(),
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut buf = [0u8; 1500];
        while sent.elapsed() < timeout {
            // SAFETY: wie oben; wir übergeben die Puffergrösse.
            let mut from: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
            let n = unsafe { libc::recvfrom(sock.0, buf.as_mut_ptr().cast(), buf.len(), 0, (&raw mut from).cast(), &raw mut len) };
            if n <= 0 {
                continue;
            }
            if from.sin_addr.s_addr != addr.sin_addr.s_addr {
                continue;
            }
            let mut data = &buf[..n as usize];
            // Raw-Sockets (und macOS) liefern den IP-Kopf mit.
            if data.len() >= 20 && data[0] >> 4 == 4 {
                let ihl = usize::from(data[0] & 0x0f) * 4;
                if data.len() < ihl + 8 {
                    continue;
                }
                data = &data[ihl..];
            }
            if data.len() >= 16 && data[0] == 0 && data[6..8] == seq.to_be_bytes() && data[8..16] == cookie {
                times.push(sent.elapsed().as_secs_f64() * 1000.0);
                break;
            }
        }
        if seq + 1 < count as u16 {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let received = times.len() as u32;
    let avg = (!times.is_empty()).then(|| times.iter().sum::<f64>() / times.len() as f64);
    Ok(PingResult { sent: count, received, avg_ms: avg })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_known() {
        // Echo Request, ID 1, Seq 1, ohne Daten: Prüfsumme 0xf7fd.
        let pkt = [8u8, 0, 0, 0, 0, 1, 0, 1];
        assert_eq!(checksum(&pkt), 0xf7fd);
    }

    #[test]
    fn ping_localhost_if_allowed() {
        if available().is_err() {
            return;
        }
        let r = ping(Ipv4Addr::LOCALHOST, 2, Duration::from_secs(1)).expect("ping");
        assert_eq!(r.received, 2, "{r:?}");
    }
}
