use uuid::Uuid;

pub fn validate_uuid(id: &str) -> Result<(), String> {
    Uuid::parse_str(id).map(|_| ()).map_err(|_| "Invalid bin ID format".to_string())
}
