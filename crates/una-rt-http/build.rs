//! Builds `una-rt`'s source with the TLS client of `una-http` (#1102): `--cfg una_rt_tls` makes
//! its `Http` operations install that client before a request.
#![allow(
    clippy::print_stdout,
    reason = "a build script speaks to cargo on stdout"
)]

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rustc-cfg=una_rt_tls");
}
