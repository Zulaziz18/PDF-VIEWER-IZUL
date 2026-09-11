//! The application's entry point.
//!
//! Everything of substance lives in the library beside this file, so that the
//! supervisor, the tile protocol and the render backend can be driven from
//! tests. What is left here is what only a binary can carry: the flag that
//! keeps a console window from appearing behind the application on Windows.

// A shipped Windows build must not open a console behind the window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    izul_app::start();
}
