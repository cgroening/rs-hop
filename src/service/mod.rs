//! Service layer: business rules over the storage ports.
//!
//! [`repo_service::RepoService`] owns the working entry list and enforces the
//! rules (validation, slug uniqueness, archive, undo); [`status_service`]
//! gathers git status synchronously or in the background.

pub mod repo_service;
pub mod status_service;
