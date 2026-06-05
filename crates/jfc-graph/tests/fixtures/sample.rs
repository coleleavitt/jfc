// Expected nodes: 4 Functions (foo, bar, baz, process), 1 Struct (Config, 4 fields),
//                 1 Enum (Status, 3 variants), 1 Trait (Processor, 2 methods), 1 Module (helpers)
// Expected edges: foo->bar (Calls), bar->baz (Calls), process uses Config (UsesType),
//                 Config implements Processor (Implements)

mod helpers {
    pub fn helper_one() -> i32 {
        42
    }
}

pub struct Config {
    pub name: String,
    pub port: u16,
    pub debug: bool,
    pub max_connections: usize,
}

pub enum Status {
    Active,
    Inactive,
    Error(String),
}

pub trait Processor {
    fn process(&self, input: &str) -> String;
    fn validate(&self) -> bool;
}

impl Processor for Config {
    fn process(&self, input: &str) -> String {
        format!("{}: {}", self.name, input)
    }

    fn validate(&self) -> bool {
        self.port > 0 && self.max_connections > 0
    }
}

pub fn foo() {
    bar();
}

fn bar() {
    baz();
}

fn baz() -> i32 {
    42
}

pub fn process(config: &Config) -> Status {
    if config.validate() {
        Status::Active
    } else {
        Status::Inactive
    }
}
