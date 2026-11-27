//! Real-sysroot integration tests for `std.net`: IPv4/IPv6/SocketAddr
//! parsing and formatting, `ToSocketAddrs` with custom runtime-dependent
//! allocators, `Slice<u8>` socket APIs (no `MutSlice`), `Reader`/`Write`
//! bounds, `NetIoWait` async effect bridge, and affine `Once` rejections.
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-std-net-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "build",
                "main.fl",
                "--entry",
                "run",
                "--output",
                "program",
                "--opt-level",
                "0",
                "--fuel",
                "10000000",
                "--build-target",
                "x86_64",
                "linux",
                "64",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .output()
            .unwrap()
    }

    fn check(&self) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "check",
                "main.fl",
                "--build-target",
                "x86_64",
                "linux",
                "64",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .output()
            .unwrap()
    }

    fn run(&self) -> Output {
        ProcessCommand::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for NativeCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn ipv4_ipv6_and_socket_addr_parsing_formatting_and_predicates() {
    let source = r#"
        use std.prelude.*;
        use std.fmt.format;
        use std.net.{
            AddrParseErr, IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6,
        };

        fn expect_invalid_ip(s: str) -> bool {
            IpAddr.parse(s) match {
                _! => false,
                error _ => true,
            }
        }

        fn expect_invalid_sock(s: str) -> bool {
            SocketAddr.parse(s) match {
                _! => false,
                error _ => true,
            }
        }

        fn run() -> bool {
            let v4 = Ipv4Addr.parse("127.0.0.1") match {
                a! => a,
                error _ => return false,
            };
            let v4_octs: builtin.Array<u8, 4> = v4.octets();
            if v4_octs.len() != 4 or v4_octs[0].* != 127 or v4_octs[3].* != 1 {
                return false;
            }
            let localhost4 = Ipv4Addr.LOCALHOST;
            if not v4.is_loopback() or not v4.ref.as(Eq).eq(localhost4.ref) {
                return false;
            }
            let priv_v4 = Ipv4Addr.parse("192.168.1.42") match {
                a! => a,
                error _ => return false,
            };
            if not priv_v4.is_private() or priv_v4.to_bits() != Ipv4Addr.from_bits(priv_v4.to_bits()).to_bits() {
                return false;
            }
            let mcast = Ipv4Addr.parse("224.0.0.1") match {
                a! => a,
                error _ => return false,
            };
            if not mcast.is_multicast() {
                return false;
            }

            let v6_loop = Ipv6Addr.parse("::1") match {
                a! => a,
                error _ => return false,
            };
            let localhost6 = Ipv6Addr.LOCALHOST;
            if not v6_loop.is_loopback() or not v6_loop.ref.as(Eq).eq(localhost6.ref) {
                return false;
            }
            let v6_doc = Ipv6Addr.parse("2001:0db8::0001") match {
                a! => a,
                error _ => return false,
            };
            let v6_fmt = format("{}", v6_doc) match {
                s! => s,
                error _ => return false,
            };
            if not v6_fmt.ref.equals_str("2001:db8::1") {
                return false;
            }
            let v6_mapped = Ipv6Addr.parse("::ffff:127.0.0.1") match {
                a! => a,
                error _ => return false,
            };
            let v6_segs: builtin.Array<u16, 8> = v6_mapped.segments();
            if v6_segs.len() != 8 or v6_segs[5].* != 65535 or v6_segs[6].* != 32512 or v6_segs[7].* != 1 {
                return false;
            }
            if localhost6.segments()[7].* != 1 {
                return false;
            }

            let sa4 = SocketAddr.parse("10.0.0.5:8080") match {
                s! => s,
                error _ => return false,
            };
            if sa4.port() != 8080 or not sa4.ip().is_ipv4() {
                return false;
            }
            let sa6 = SocketAddr.parse("[2001:db8::1]:443") match {
                s! => s,
                error _ => return false,
            };
            let sa6_fmt = format("{}", sa6) match {
                s! => s,
                error _ => return false,
            };
            if sa6.port() != 443 or not sa6.ip().is_ipv6() or not sa6_fmt.ref.equals_str("[2001:db8::1]:443") {
                return false;
            }

            -- Rejection cases
            if not expect_invalid_ip("")
                or not expect_invalid_ip("256.0.0.1")
                or not expect_invalid_ip("01.2.3.4")
                or not expect_invalid_ip("1.2.3")
                or not expect_invalid_ip("1.2.3.4.5")
                or not expect_invalid_ip("1::2::3")
                or not expect_invalid_ip("1:2:3:4:5:6:7:8:9")
                or not expect_invalid_ip("gggg::1")
                or not expect_invalid_ip("1:2:3:4:5:6:7:")
            {
                return false;
            }
            if not expect_invalid_sock("127.0.0.1")
                or not expect_invalid_sock("127.0.0.1:65536")
                or not expect_invalid_sock("[::1")
                or not expect_invalid_sock("[::1]:")
            {
                return false;
            }
            true
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let out = case.run();
    assert_success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn to_socket_addrs_respects_custom_allocator_and_drops_cleanly() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{Allocator, FixedBufferAllocator, Layout, SystemAllocator};
        use std.net.{Ipv4Addr, SocketAddr, ToSocketAddrs};

        fn resolve_with_fba(fba: *FixedBufferAllocator) -> bool {
            do {
                let raw: str = "127.0.0.1:9001";
                let addrs = raw.ref.as(ToSocketAddrs).to_socket_addrs<.A = FixedBufferAllocator, .allocator = fba>() match {
                    v! => v,
                    error _ => return false,
                };
                if addrs.ref.len() != 1 or fba.live_allocations() == 0 {
                    return false;
                }
                let first = addrs.ref.get(0) match {
                    s? => s,
                    null => return false,
                };
                if first.port() != 9001 {
                    return false;
                }
            }
            fba.live_allocations() == 0
        }

        fn run() -> bool {
            let layout = Layout.new(2048, 16) match {
                l? => l,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                b! => b,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let ok = resolve_with_fba(fba.ref);
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            ok
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let out = case.run();
    assert_success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn std_net_socket_types_slice_apis_and_oq_boundaries_check_cleanly() {
    let valid_net = r#"
        use std.prelude.*;
        use std.io.{Reader, Write};
        use std.net.{
            Incoming, NetErr, NetIoWait, Shutdown, TcpListener, TcpStream, UdpSocket, UnixListener,
            UnixStream,
        };
        use std.sync.{Send, Sync};

        fn require_reader(r: *R) where R :- Reader {}
        fn require_writer(w: *W) where W :- Write {}
        fn require_send_sync(v: T) where T :- Send + Sync {}

        fn use_async_stream(stream: *TcpStream, buf: Slice<u8>) -> #NetIoWait !NetErr usize {
            stream.read_async(buf)
        }

        fn poll_incoming(inc: *Incoming) -> ?(!NetErr TcpStream) {
            inc.next()
        }

        fn check_types(tcp: TcpStream, unix_s: UnixStream, udp: UdpSocket, listener: TcpListener, ulistener: UnixListener) {
            let inc = listener.ref.incoming();
            let _ = poll_incoming(inc.ref);
            require_reader(tcp.ref);
            require_writer(tcp.ref);
            require_reader(unix_s.ref);
            require_writer(unix_s.ref);
            require_send_sync(tcp);
            require_send_sync(unix_s);
            require_send_sync(udp);
            require_send_sync(listener);
            require_send_sync(ulistener);
        }

        fn run() -> bool { true }
    "#;
    assert_success(&NativeCase::new(valid_net).check());

    let rejections = [
        (
            "OQ-08-07: MutSlice must not exist",
            r#"
                use std.collections.MutSlice;
                fn run() -> bool { true }
            "#,
        ),
        (
            "TcpStream must not be Copy",
            r#"
                use std.net.TcpStream;
                fn req(v: T) where T :- Copy {}
                fn bad(s: TcpStream) { req(s); }
            "#,
        ),
        (
            "Tier-1 boundary: HttpClient must not exist in std.net",
            r#"
                use std.net.HttpClient;
                fn run() -> bool { true }
            "#,
        ),
    ];

    for (label, source) in rejections {
        let out = NativeCase::new(source).check();
        assert!(
            !out.status.success(),
            "expected failure for `{label}`, but check succeeded"
        );
    }
}
