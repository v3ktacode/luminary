// src/client/endpoints/comments.rs
//
// Comments on UGC (user-generated content — rooms, looks, etc.) go through
// a separate GraphQL API rather than the REST-style JSON endpoints
//
// The GraphQL gateway is region-aware just like every other endpoint —
// EU accounts are routed to `eu.mspapis.com`, US/CA accounts to
// `us.mspapis.com` — via `MspConfig::comments_endpoint_regional`. The
// region itself comes from the logged-in session (`MspSession::region`),
// set at login time, so callers never need to specify it manually.

use std::sync::Arc;

use serde::Deserialize;

use super::super::http::{build_headers, decode_response, ContentType};
use crate::config::MspConfig;
use crate::errors::{MspError, Result};
use crate::models::SentComment;
use crate::session::SessionStore;

use wreq::Client;

/// Short label attached to every `MspError` raised from this module.
const EP: &str = "comments";

const ORIGIN: &str = "https://moviestarplanet2.com";
const REFERER: &str = "https://moviestarplanet2.com/";

/// The only entity type this crate posts comments against.
const ENTITY_TYPE_UGC: &str = "UGC";

/// GraphQL mutation for posting a comment onto a thread.
///
/// `threadId` identifies the UGC item being commented on. `author` is the
/// commenting profile's id — the server presumably cross-checks this
/// against the bearer token's identity rather than trusting it blindly.
const POST_COMMENT_MUTATION: &str = "\
mutation SendComment($entityType: String!, $threadId: String!, $text: String!, $author: String!) {
  postComment(input: { entityType: $entityType, threadId: $threadId, text: $text, author: $author }) {
    success
    error
    comment {
      commentId
      created
      author
      text
    }
  }
}";

pub struct CommentsEndpoint<'c> {
    pub(crate) http: &'c Client,
    pub(crate) session: &'c SessionStore,
    pub(crate) config: Arc<MspConfig>,
}

impl<'c> CommentsEndpoint<'c> {
    /// Posts a comment onto a UGC thread and returns the comment as stored
    /// by the server (with its assigned `commentId` and `created` timestamp).
    ///
    /// `thread_id` is the id of the UGC item being commented on
    ///
    /// The request is routed to the GraphQL gateway matching the logged-in
    /// session's region (EU or US) automatically.
    #[tracing::instrument(name = "comments.post", skip(self, text), fields(thread_id = %thread_id))]
    pub async fn post(&self, thread_id: &str, text: &str) -> Result<SentComment> {
        let session = self.session.get().await?;
        let url = self.config.comments_endpoint_regional(&session.region);

        let payload = serde_json::json!({
            "query": POST_COMMENT_MUTATION,
            "variables": {
                "entityType": ENTITY_TYPE_UGC,
                "threadId":   thread_id,
                "text":       text,
                "author":     session.profile_id,
            }
        });

        let response = self
            .http
            .post(&url)
            .headers(build_headers(
                ContentType::Json,
                Some(&session.bearer()),
                ORIGIN,
                REFERER,
            ))
            .json(&payload)
            .send()
            .await
            .map_err(|e| MspError::from_wreq(e, EP))?;

        // `decode_response` already handles the transport-level concerns
        // (429, non-2xx status, top-level `{"errors": [...]}` — which is
        // exactly the shape GraphQL uses for query-level failures). What's
        // left for `into_sent_comment` to check is the *mutation-level*
        // outcome: `postComment.success` / `postComment.error`, which is
        // specific to this one mutation and isn't something a generic
        // decoder could know about.
        let envelope: GraphQlResponse = decode_response(response, EP).await?;
        envelope.into_sent_comment()
    }
}

// ---------------------------------------------------------------------------
// GraphQL response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GraphQlResponse {
    #[serde(default)]
    data: Option<ResponseData>,

    /// In practice this is almost always empty by the time it reaches here:
    /// `decode_response` already intercepts a non-empty top-level `errors`
    /// array and turns it into `MspError::GraphQl` before this struct is
    /// even deserialized. This field is kept as a defensive fallback in
    /// case that shared logic ever changes, not because it's expected to
    /// fire in normal operation.
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

#[derive(Debug, Deserialize)]
struct ResponseData {
    #[serde(rename = "postComment")]
    post_comment: PostCommentResult,
}

#[derive(Debug, Deserialize)]
struct PostCommentResult {
    success: bool,
    error: Option<String>,
    comment: Option<SentComment>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
}

impl GraphQlResponse {
    /// Turns the raw GraphQL envelope into either a `SentComment` or the
    /// most specific error we can extract from it.
    ///
    /// There are three distinct failure shapes to account for here, from
    /// most to least specific:
    ///   1. Query-level GraphQL errors (`self.errors` — see the note above).
    ///   2. Mutation-level failure (`postComment.success == false`).
    ///   3. Malformed/unexpected response shape (missing `data`, or
    ///      `success == true` with no `comment` attached) — these shouldn't
    ///      happen against a well-behaved server, so they're surfaced as
    ///      generic API errors rather than given their own variant.
    fn into_sent_comment(self) -> Result<SentComment> {
        if !self.errors.is_empty() {
            let message = self
                .errors
                .into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; ");
            return Err(MspError::graphql(EP, message));
        }

        let result = self.data.map(|d| d.post_comment).ok_or_else(|| {
            MspError::api(
                EP,
                422,
                "GraphQL response contained neither 'data' nor 'errors'",
            )
        })?;

        if !result.success {
            let message = result
                .error
                .unwrap_or_else(|| "postComment reported success = false".into());
            return Err(MspError::api(EP, 400, message));
        }

        result
            .comment
            .ok_or_else(|| MspError::api(EP, 422, "postComment succeeded but returned no comment"))
    }
}