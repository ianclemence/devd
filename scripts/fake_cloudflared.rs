// A stand-in for cloudflared, to exercise devd's share links without opening a real tunnel.
// It prints the banner a quick tunnel prints, with a made-up address, then stays up.
//
//   rustc -O scripts/fake_cloudflared.rs -o <home>/tools/cloudflared/9.9.9/cloudflared.exe
fn main() {
    eprintln!("INF Requesting new quick Tunnel on trycloudflare.com...");
    eprintln!("INF |  https://fake-tunnel-test.trycloudflare.com  |");
    eprintln!("INF args: {}", std::env::args().skip(1).collect::<Vec<_>>().join(" "));
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
