use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::models::LoggedRequest;

#[derive(Clone)]
pub struct AppState {
   pub bins: Arc<Mutex<HashMap<String, Vec<LoggedRequest>>>>,
}

impl AppState {
   pub fn new() -> Self {
      AppState {
         bins: Arc::new(Mutex::new(HashMap::new())),
      }
   }
}