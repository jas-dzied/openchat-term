use env_logger::Env;
use openchat::ocserver;

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    ocserver::start_server("127.0.0.1:8080").unwrap();
}
