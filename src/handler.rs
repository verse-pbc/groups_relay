use crate::app_state::HttpServerState;
use crate::groups::Invite;
use axum::{extract::State, http::StatusCode, response::Json};
use nostr_sdk::nips::nip11::RelayInformationDocument;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

#[derive(Serialize)]
pub struct GroupResponse {
    id: String,
    name: String,
    about: Option<String>,
    picture: Option<String>,
    private: bool,
    closed: bool,
    members: Vec<MemberResponse>,
    invites: HashMap<String, Invite>,
    join_requests: Vec<String>,
    created_at: u64,
    updated_at: u64,
}

#[derive(Serialize)]
struct MemberResponse {
    pubkey: String,
    roles: Vec<String>,
}

pub async fn handle_get_groups(
    State(state): State<Arc<HttpServerState>>,
) -> Result<Json<Vec<GroupResponse>>, StatusCode> {
    match try_get_groups(state) {
        Ok(groups) => Ok(Json(groups)),
        Err(e) => {
            error!("Error fetching groups: {:?}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

fn try_get_groups(
    state: Arc<HttpServerState>,
) -> Result<Vec<GroupResponse>, Box<dyn std::error::Error>> {
    let groups = state
        .groups
        .iter()
        .map(|group| {
            let group = group.value();
            GroupResponse {
                id: group.id.clone(),
                name: group.metadata.name.clone(),
                about: group.metadata.about.clone(),
                picture: group.metadata.picture.clone(),
                private: group.metadata.private,
                closed: group.metadata.closed,
                members: group
                    .members
                    .values()
                    .map(|member| MemberResponse {
                        pubkey: member.pubkey.to_string(),
                        roles: member.roles.iter().map(|r| r.to_string()).collect(),
                    })
                    .collect(),
                invites: group.invites.clone(),
                join_requests: group
                    .join_requests
                    .iter()
                    .map(|pk| pk.to_string())
                    .collect(),
                created_at: group.created_at.as_u64(),
                updated_at: group.updated_at.as_u64(),
            }
        })
        .collect();

    Ok(groups)
}

pub async fn handle_nostr_json(
    State(_state): State<Arc<HttpServerState>>,
) -> Json<RelayInformationDocument> {
    let relay_info = RelayInformationDocument {
        name: Some("Nostr Groups Relay".to_string()),
        description: Some(
            "A specialized relay implementing NIP-29 for Nostr group management".to_string(),
        ),
        supported_nips: Some(vec![1, 11, 29, 42]),
        software: Some("groups_relay".to_string()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        ..Default::default()
    };

    Json(relay_info)
}

pub async fn handle_health() -> &'static str {
    "OK"
}
