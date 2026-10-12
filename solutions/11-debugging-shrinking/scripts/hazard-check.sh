#!/usr/bin/env bash
# Part 3 of the Module 3 task: prove that each hazard, pasted into the core
# crate, fails to compile or fails lint. For every snippet below we add a
# module to kv-core, run `cargo clippy`, and require it to FAIL. The crate
# is restored afterwards whatever happens.
set -uo pipefail
cd "$(dirname "$0")/.."

CORE=crates/kv-core/src
LIB=$CORE/lib.rs
cp "$LIB" "$LIB.orig"
trap 'mv "$LIB.orig" "$LIB"; rm -f "$CORE/hazard_probe.rs"' EXIT

failures=0
probe() {
  local name="$1" body="$2"
  printf '%s\n' "$body" > "$CORE/hazard_probe.rs"
  printf '\npub mod hazard_probe;\n' >> "$LIB"
  if cargo clippy -q -p kv-core 2>/dev/null; then
    echo "FAIL  $name: compiled and passed lint, hazard is reachable from kv-core"
    failures=$((failures + 1))
  else
    echo "ok    $name: rejected"
  fi
  cp "$LIB.orig" "$LIB"
}

probe "Instant::now"        'pub fn f() -> u64 { std::time::Instant::now().elapsed().as_nanos() as u64 }'
probe "SystemTime::now"     'pub fn f() -> bool { std::time::SystemTime::now() > std::time::UNIX_EPOCH }'
probe "thread::spawn"       'pub fn f() { std::thread::spawn(|| {}).join().unwrap(); }'
probe "thread::sleep"       'pub fn f() { std::thread::sleep(core::time::Duration::from_millis(1)); }'
probe "HashMap"             'pub fn f() -> usize { let m: std::collections::HashMap<u32, u32> = Default::default(); m.len() }'
probe "hashbrown via alloc" 'extern crate alloc; pub fn f() -> usize { let m: alloc::collections::HashMap<u32,u32> = Default::default(); m.len() }'
probe "env::var"            'pub fn f() -> bool { std::env::var("HOME").is_ok() }'
probe "fs::read_dir"        'pub fn f() -> bool { std::fs::read_dir(".").is_ok() }'
probe "net"                 'pub fn f() -> bool { std::net::TcpStream::connect("127.0.0.1:1").is_ok() }'
probe "Mutex"               'pub fn f() -> u32 { let m = std::sync::Mutex::new(1u32); *m.lock().unwrap() }'
probe "OnceLock"            'pub static S: std::sync::OnceLock<u32> = std::sync::OnceLock::new(); pub fn f() -> u32 { *S.get_or_init(|| 1) }'
probe "static mut"          'pub static mut COUNTER: u64 = 0; pub fn f() -> u64 { unsafe { COUNTER += 1; COUNTER } }'
probe "unsafe"              'pub fn f(p: *const u8) -> u8 { unsafe { *p } }'
probe "pointer as key"      'extern crate alloc; pub fn f(a: &u8, b: &u8) -> bool { (a as *const u8 as usize) < (b as *const u8 as usize) }'
probe "f64::sin (libm)"     'pub fn f(x: f64) -> f64 { x.sin() }'
probe "println"             'pub fn f() { println!("hello"); }'

if [ "$failures" -ne 0 ]; then
  echo "$failures hazard(s) reachable from kv-core"
  exit 1
fi
echo "all hazards rejected by kv-core"
