use env_logger::Env;
use openchat::ocserver;

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    ocserver::start_server("192.168.0.164:8080").unwrap();
}
