use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct Connection {}

pub struct ConnectionManager {
    map: HashMap<u128, Arc<Mutex<Connection>>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}
