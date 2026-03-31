use crate::constants::SESSION_CSRF_TOKENS_KEY;
use crate::errors::SiteError;
use std::collections::HashMap;
use tower_sessions::Session;
use uuid::Uuid;

pub(crate) trait SessionCsrfExt {
    async fn issue_csrf_token(&self, scope: &str) -> Result<String, SiteError>;
    async fn validate_csrf_token(
        &self,
        scope: &str,
        submitted_token: &str,
    ) -> Result<(), SiteError>;
}

impl SessionCsrfExt for Session {
    async fn issue_csrf_token(&self, scope: &str) -> Result<String, SiteError> {
        let mut tokens = self
            .get::<HashMap<String, String>>(SESSION_CSRF_TOKENS_KEY)
            .await
            .map_err(|_| SiteError::internal("failed to read csrf state"))?
            .unwrap_or_default();
        let token = Uuid::now_v7().to_string();
        tokens.insert(scope.to_string(), token.clone());
        self.insert(SESSION_CSRF_TOKENS_KEY, tokens)
            .await
            .map_err(|_| SiteError::internal("failed to persist csrf state"))?;
        Ok(token)
    }

    async fn validate_csrf_token(
        &self,
        scope: &str,
        submitted_token: &str,
    ) -> Result<(), SiteError> {
        if submitted_token.trim().is_empty() {
            return Err(SiteError::BadRequest("missing csrf token".to_string()));
        }

        let mut tokens = self
            .get::<HashMap<String, String>>(SESSION_CSRF_TOKENS_KEY)
            .await
            .map_err(|_| SiteError::internal("failed to read csrf state"))?
            .unwrap_or_default();
        let Some(expected_token) = tokens.get(scope) else {
            return Err(SiteError::BadRequest("missing csrf state".to_string()));
        };
        if expected_token != submitted_token {
            return Err(SiteError::BadRequest("invalid csrf token".to_string()));
        }

        tokens.remove(scope);
        self.insert(SESSION_CSRF_TOKENS_KEY, tokens)
            .await
            .map_err(|_| SiteError::internal("failed to persist csrf state"))?;
        Ok(())
    }
}
