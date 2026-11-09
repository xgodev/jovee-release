//! Responsibility: parses `--name value` options and positional arguments.

use std::collections::HashMap;

pub struct Args {
    named: HashMap<String, String>,
    pub positional: Vec<String>,
}

impl Args {
    pub fn parse(argv: &[String]) -> Args {
        let mut named = HashMap::new();
        let mut positional = Vec::new();
        let mut it = argv.iter();
        while let Some(a) = it.next() {
            match a.strip_prefix("--") {
                Some(name) => {
                    named.insert(name.to_string(), it.next().cloned().unwrap_or_default());
                }
                None => positional.push(a.clone()),
            }
        }
        Args { named, positional }
    }

    pub fn opt(&self, name: &str) -> Option<&str> {
        self.named.get(name).map(String::as_str).filter(|v| !v.is_empty())
    }

    pub fn req(&self, name: &str) -> Result<&str, String> {
        self.opt(name).ok_or_else(|| format!("missing --{name}"))
    }
}
