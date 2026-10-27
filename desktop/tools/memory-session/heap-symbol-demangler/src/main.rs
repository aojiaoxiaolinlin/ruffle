use std::io::{self, BufRead, Write};

fn main() {
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        if let Some((module, symbol)) = line.split_once(' ') {
            if let Ok(name) = rustc_demangle::try_demangle(symbol) {
                writeln!(out, "{module} {name:#}").unwrap();
                continue;
            }
        }
        writeln!(out, "{line}").unwrap();
    }
}
