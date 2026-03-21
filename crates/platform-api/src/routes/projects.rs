use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use api_anything_common::error::AppError;
use api_anything_common::models::SourceType;
use api_anything_metadata::MetadataRepo;
use serde::Deserialize;
use uuid::Uuid;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: String,
    pub owner: String,
    pub source_type: SourceType,
}

pub async fn create_project(
    State(state): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<impl IntoResponse, AppError> {
    let project = state.repo.create_project(&req.name, &req.description, &req.owner, req.source_type).await?;
    Ok((StatusCode::CREATED, Json(project)))
}

pub async fn get_project(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let project = state.repo.get_project(id).await?;
    Ok(Json(project))
}

pub async fn list_projects(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let projects = state.repo.list_projects().await?;
    Ok(Json(projects))
}

pub async fn delete_project(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    state.repo.delete_project(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
