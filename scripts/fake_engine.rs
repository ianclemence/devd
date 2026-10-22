// A stand-in for postgres, mongod, memcached and their tools, for exercising devd's service
// plumbing (adopt, start, health, stop, data directories) where the real programs are not
// installed. It is not any of them: it prints a version, listens on the port it is told to,
// and answers the few client calls devd makes with fixed text.
//
//   rustc -O scripts/fake_engine.rs -o <folder>/bin/postgres.exe   (and copies named initdb,
//   pg_ctl, psql, pg_dump, mongod, memcached)
use std::io::Write;
use std::net::TcpListener;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let me = std::path::Path::new(&args[0]).file_stem().unwrap().to_string_lossy().to_lowercase();
    let flag = |names: &[&str]| args.windows(2).find(|w| names.contains(&w[0].as_str())).map(|w| w[1].clone());
    match me.as_str() {
        "postgres" | "mongod" | "memcached" => {
            if args.iter().any(|a| a == "--version" || a == "-V") {
                match me.as_str() {
                    "postgres" => println!("postgres (PostgreSQL) 16.4"),
                    "mongod" => println!("db version v7.0.5"),
                    _ => println!("memcached 1.6.99"),
                }
                return;
            }
            let port = flag(&["-p", "--port"]).unwrap_or_else(|| "0".into());
            let listener = TcpListener::bind(format!("127.0.0.1:{port}")).expect("bind");
            for stream in listener.incoming() {
                drop(stream);
            }
        }
        "initdb" => {
            let dir = flag(&["-D"]).expect("-D");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(std::path::Path::new(&dir).join("PG_VERSION"), "16\n").unwrap();
            println!("fake cluster created in {dir}");
        }
        "pg_ctl" => println!("fake pg_ctl: {}", args[1..].join(" ")),
        "psql" => {
            let sql = flag(&["-c"]).unwrap_or_default();
            if sql.contains("FROM pg_database") {
                println!("app\nshop");
            } else if sql.contains("count(*)") {
                println!("3");
            } else if sql.starts_with("CREATE DATABASE") && sql.contains("fail") {
                eprintln!("ERROR: refused");
                std::process::exit(1);
            }
            if let Some(file) = flag(&["-f"]) {
                println!("ran {file}");
            }
        }
        "pg_dump" => {
            let out = flag(&["-f"]).expect("-f");
            let mut f = std::fs::File::create(out).unwrap();
            writeln!(f, "-- fake dump\nCREATE TABLE t (id int);").unwrap();
        }
        other => {
            eprintln!("fake_engine does not know {other}");
            std::process::exit(2);
        }
    }
}
