use uuid::Uuid;

pub fn validate_uuid(id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(id).map_err(|_| "Invalid UUID format".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_uuid_returns_ok() {
        let id = Uuid::new_v4().to_string();
        assert!(validate_uuid(&id).is_ok());
    }

    #[test]
    fn invalid_uuid_returns_err() {
        let result = validate_uuid("not-a-uuid");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Invalid UUID format");
    }
}