use axum::{
    extract::{Request, State},
    http::{header, HeaderName, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use moka::future::Cache;
use serde::Serialize;
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::state::AppState;

/// Why a cache load did not produce a page.
enum LoadErr {
    Missing,
    Db(sqlx::Error),
}

#[derive(Clone)]
pub struct PublicCache {
    /// home, header, footer, breaking news, categories, tags, articles
    pages: Cache<String, Bytes>,
    /// free-text searches: small and short-lived so junk queries can't evict real pages
    search: Cache<String, Bytes>,
    /// page views waiting to be written to the database
    views: Arc<Mutex<HashMap<String, u32>>>,

    /// one comment per client every few seconds
    comment_gate: Cache<String, ()>,
}

fn reply(bytes: Bytes, fresh: bool) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "public, max-age=5"),
            (HeaderName::from_static("x-cache"), if fresh { "MISS" } else { "HIT" }),
        ],
        bytes,
    )
        .into_response()
}

impl PublicCache {
    pub fn new() -> Self {
        Self {
            pages: Cache::builder()
                .max_capacity(2_000)
                .time_to_live(Duration::from_secs(60)) // safety net; real updates come from clear()
                .support_invalidation_closures()
                .build(),
            search: Cache::builder()
                .max_capacity(500)
                .time_to_live(Duration::from_secs(30))
                .build(),
            views: Arc::new(Mutex::new(HashMap::new())),
            comment_gate: Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(15))
                .build(),
        }
    }

    /// Drop every cached page and search. Call after anything that changes public content.
    pub fn clear(&self) {
        self.pages.invalidate_all();
        self.search.invalidate_all();
    }

    /// Remember one page view; flushed to the database by the background task in main.rs.
    pub fn count_view(&self, slug: &str) {
        let mut m = self.views.lock().unwrap_or_else(|e| e.into_inner());
        *m.entry(slug.to_string()).or_insert(0) += 1;
    }

    /// Take all buffered views (leaves the buffer empty).
    pub fn take_views(&self) -> HashMap<String, u32> {
        let mut m = self.views.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *m)
    }

        /// Raw lookup for the generic GET cache.
    pub async fn get_raw(&self, key: &str) -> Option<Bytes> {
        self.pages.get(key).await
    }

    pub async fn put_raw(&self, key: String, bytes: Bytes) {
        self.pages.insert(key, bytes).await;
    }

    /// Drop every cached comment page of one article (called after a new comment).
    pub fn forget_comments(&self, slug: &str) {
        let prefix = format!("comments|{slug}|");
        let _ = self.pages.invalidate_entries_if(move |k, _| k.starts_with(&prefix));
    }

    /// false if this client already posted a comment in the last few seconds.
    pub async fn comment_allowed(&self, ip: &str) -> bool {
        self.comment_gate.entry(ip.to_string()).or_insert(()).await.is_fresh()
    }

    /// Shared loader: return the cached JSON for `key`, or run `load` and cache it.
    /// Concurrent misses for the same key run `load` only once. Errors are never cached.
    async fn load_json<T, Fut>(cache: &Cache<String, Bytes>, key: String, load: Fut) -> Result<Response, sqlx::Error>
    where
        T: Serialize,
        Fut: Future<Output = Result<T, sqlx::Error>>,
    {
        let entry = cache
            .entry(key)
            .or_try_insert_with(async {
                let value = load.await?;
                serde_json::to_vec(&value)
                    .map(Bytes::from)
                    .map_err(|e| sqlx::Error::Decode(Box::new(e)))
            })
            .await
            .map_err(|e| match Arc::try_unwrap(e) {
                Ok(inner) => inner,
                Err(shared) => sqlx::Error::Protocol(shared.to_string()),
            })?;

        let fresh = entry.is_fresh();
        Ok(reply(entry.into_value(), fresh))
    }

    /// Cached JSON in the main page cache.
    pub async fn json<T, Fut>(&self, key: String, load: Fut) -> Result<Response, sqlx::Error>
    where
        T: Serialize,
        Fut: Future<Output = Result<T, sqlx::Error>>,
    {
        Self::load_json(&self.pages, key, load).await
    }

    /// Cached JSON in the small, short-lived search cache.
    pub async fn search_json<T, Fut>(&self, key: String, load: Fut) -> Result<Response, sqlx::Error>
    where
        T: Serialize,
        Fut: Future<Output = Result<T, sqlx::Error>>,
    {
        Self::load_json(&self.search, key, load).await
    }

    /// Like `json`, but for lookups that can find nothing: `None` is returned
    /// as `Ok(None)` and is never cached (so 404s can't fill the cache).
    pub async fn json_opt<T, Fut>(&self, key: String, load: Fut) -> Result<Option<Response>, sqlx::Error>
    where
        T: Serialize,
        Fut: Future<Output = Result<Option<T>, sqlx::Error>>,
    {
        let result = self
            .pages
            .entry(key)
            .or_try_insert_with(async {
                match load.await {
                    Ok(Some(value)) => serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .map_err(|e| LoadErr::Db(sqlx::Error::Decode(Box::new(e)))),
                    Ok(None) => Err(LoadErr::Missing),
                    Err(e) => Err(LoadErr::Db(e)),
                }
            })
            .await;

        match result {
            Ok(entry) => {
                let fresh = entry.is_fresh();
                Ok(Some(reply(entry.into_value(), fresh)))
            }
            Err(e) => match Arc::try_unwrap(e) {
                Ok(LoadErr::Missing) => Ok(None),
                Ok(LoadErr::Db(inner)) => Err(inner),
                Err(shared) => match &*shared {
                    LoadErr::Missing => Ok(None),
                    LoadErr::Db(inner) => Err(sqlx::Error::Protocol(inner.to_string())),
                },
            },
        }
    }
}

/// Admin router layer: after any successful write, clear the public cache.
pub async fn purge_on_admin_write(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let is_write = matches!(*req.method(), Method::POST | Method::PUT | Method::PATCH | Method::DELETE);
    let res = next.run(req).await;
    if is_write && (res.status().is_success() || res.status().is_redirection()) {
        state.cache.clear(); // runs after the handler, so the DB write has already committed
    }
    res
}

pub async fn cached_get(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if req.method() != Method::GET {
        return next.run(req).await;
    }
    let key = format!("get|{}", req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/"));
    if key.len() > 200 {
        return next.run(req).await; // never build huge keys from junk URLs
    }
    if let Some(bytes) = state.cache.get_raw(&key).await {
        return reply(bytes, false);
    }

    let res = next.run(req).await;
    if res.status() != StatusCode::OK {
        return res;
    }
    let (parts, body) = res.into_parts();
    match axum::body::to_bytes(body, 2 * 1024 * 1024).await {
        Ok(bytes) => {
            state.cache.put_raw(key, bytes.clone()).await;
            reply(bytes, true)
        }
        Err(_) => Response::from_parts(parts, axum::body::Body::empty()),
    }
}