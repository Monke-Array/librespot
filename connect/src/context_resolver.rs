use crate::{
    core::{Error, Session, error::ErrorKind, http_client::HttpClientError},
    protocol::{
        autoplay_context_request::AutoplayContextRequest, context::Context,
        transfer_state::TransferState,
    },
    state::{ConnectState, context::ContextType},
};
use std::{
    cmp::PartialEq,
    collections::{HashMap, VecDeque},
    fmt::{Display, Formatter},
    hash::Hash,
    time::Duration,
};
use thiserror::Error as ThisError;
use tokio::time::Instant;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum Resolve {
    Uri(String),
    Context(Context),
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(super) enum ContextAction {
    Append,
    Replace,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(super) struct ResolveContext {
    resolve: Resolve,
    fallback: Option<String>,
    update: ContextType,
    action: ContextAction,
}

impl ResolveContext {
    fn append_context(uri: impl Into<String>) -> Self {
        Self {
            resolve: Resolve::Uri(uri.into()),
            fallback: None,
            update: ContextType::Default,
            action: ContextAction::Append,
        }
    }

    pub fn from_uri(
        uri: impl Into<String>,
        fallback: impl Into<String>,
        update: ContextType,
        action: ContextAction,
    ) -> Self {
        let fallback_uri = fallback.into();
        Self {
            resolve: Resolve::Uri(uri.into()),
            fallback: (!fallback_uri.is_empty()).then_some(fallback_uri),
            update,
            action,
        }
    }

    pub fn from_context(context: Context, update: ContextType, action: ContextAction) -> Self {
        Self {
            resolve: Resolve::Context(context),
            fallback: None,
            update,
            action,
        }
    }

    /// the uri which should be used to resolve the context, might not be the context uri
    fn resolve_uri(&self) -> Option<&str> {
        // it's important to call this always, or at least for every ResolveContext
        // otherwise we might not even check if we need to fallback and just use the fallback uri
        match self.resolve {
            Resolve::Uri(ref uri) => ConnectState::valid_resolve_uri(uri),
            Resolve::Context(ref ctx) => {
                ConnectState::find_valid_uri(ctx.uri.as_deref(), ctx.pages.first())
            }
        }
        .or(self.fallback.as_deref())
    }

    /// the actual context uri
    fn context_uri(&self) -> &str {
        match self.resolve {
            Resolve::Uri(ref uri) => uri,
            Resolve::Context(ref ctx) => ctx.uri.as_deref().unwrap_or_default(),
        }
    }
}

impl Display for ResolveContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "resolve_uri: <{:?}>, context_uri: <{}>, update: <{:?}>",
            self.resolve_uri(),
            self.context_uri(),
            self.update,
        )
    }
}

#[derive(Debug, ThisError)]
enum ContextResolverError {
    #[error("no next context to resolve")]
    NoNext,
    #[error("tried appending context with {0} pages")]
    UnexpectedPagesSize(usize),
    #[error("tried resolving not allowed context: {0:?}")]
    NotAllowedContext(String),
}

impl From<ContextResolverError> for Error {
    fn from(value: ContextResolverError) -> Self {
        Error::failed_precondition(value)
    }
}

pub struct ContextResolver {
    session: Session,
    queue: VecDeque<ResolveContext>,
    unavailable_contexts: HashMap<ResolveContext, ContextFailure>,
}

// time after which an unavailable context is retried
const RETRY_UNAVAILABLE: Duration = Duration::from_secs(3600);
const RETRY_TRANSIENT: Duration = Duration::from_secs(2);
const RETRY_SESSION: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ContextFailureKind {
    Permanent,
    Transient,
    SessionInvalid,
}

#[derive(Clone, Copy, Debug)]
struct ContextFailure {
    last_try: Instant,
    retry_after: Duration,
}

impl ContextResolver {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            queue: VecDeque::new(),
            unavailable_contexts: HashMap::new(),
        }
    }

    pub(crate) fn set_session(&mut self, session: Session) {
        self.session = session;
    }

    pub fn add(&mut self, resolve: ResolveContext) {
        let failure = self.unavailable_contexts.get(&resolve).copied();

        let elapsed = failure.map(|failure| Instant::now().duration_since(failure.last_try));
        let failure = if matches!((failure, elapsed), (Some(failure), Some(elapsed)) if elapsed >= failure.retry_after)
        {
            let _ = self.unavailable_contexts.remove(&resolve);
            debug!(
                "context backoff elapsed after {}s; allowing resolution again",
                elapsed.expect("checked by condition").as_secs()
            );
            None
        } else {
            failure
        };

        if failure.is_some() {
            debug!("tried loading unavailable context: {resolve}");
            return;
        } else if self.queue.contains(&resolve) {
            debug!("update for {resolve} is already added");
            return;
        } else {
            trace!(
                "added {} to resolver queue",
                resolve.resolve_uri().unwrap_or(resolve.context_uri())
            )
        }

        self.queue.push_back(resolve)
    }

    pub fn add_list(&mut self, resolve: Vec<ResolveContext>) {
        for resolve in resolve {
            self.add(resolve)
        }
    }

    pub fn remove_used_and_invalid(&mut self) {
        if let Some((_, _, remove)) = self.find_next() {
            let _ = self.queue.drain(0..remove); // remove invalid
        }
        self.queue.pop_front(); // remove used
    }

    pub fn clear(&mut self) {
        self.queue = VecDeque::new()
    }

    fn find_next(&self) -> Option<(&ResolveContext, &str, usize)> {
        for idx in 0..self.queue.len() {
            let next = self.queue.get(idx)?;
            match next.resolve_uri() {
                None => {
                    warn!("skipped {idx} because of invalid resolve_uri: {next}");
                    continue;
                }
                Some(uri) => return Some((next, uri, idx)),
            }
        }
        None
    }

    pub fn has_next(&self) -> bool {
        !self.session.is_invalid() && self.find_next().is_some()
    }

    pub async fn get_next_context(
        &self,
        recent_track_uri: impl Fn() -> Vec<String>,
    ) -> Result<Context, Error> {
        let (next, resolve_uri, _) = self.find_next().ok_or(ContextResolverError::NoNext)?;

        if let Some(failure) = self.unavailable_contexts.get(next) {
            let retry_at = failure.last_try + failure.retry_after;
            if retry_at > Instant::now() {
                tokio::time::sleep_until(retry_at).await;
            }
        }

        match next.update {
            ContextType::Default => {
                let mut ctx = self.session.spclient().get_context(resolve_uri).await;
                if let Ok(ctx) = ctx.as_mut() {
                    ctx.uri = Some(next.context_uri().to_string());
                    ctx.url = ctx.uri.as_ref().map(|s| format!("context://{s}"));
                }

                ctx
            }
            ContextType::Autoplay => {
                if resolve_uri.contains("spotify:show:") || resolve_uri.contains("spotify:episode:")
                {
                    // autoplay is not supported for podcasts
                    Err(ContextResolverError::NotAllowedContext(
                        resolve_uri.to_string(),
                    ))?
                }

                let request = AutoplayContextRequest {
                    context_uri: Some(resolve_uri.to_string()),
                    recent_track_uri: recent_track_uri(),
                    ..Default::default()
                };
                self.session.spclient().get_autoplay_context(&request).await
            }
        }
    }

    pub fn classify_failure(&self, error: &Error) -> ContextFailureKind {
        if self.session.is_invalid() || error.kind == ErrorKind::Unauthenticated {
            return ContextFailureKind::SessionInvalid;
        }

        if let Some(HttpClientError::StatusCode(status)) =
            error.error.downcast_ref::<HttpClientError>()
        {
            return match status.as_u16() {
                400 | 404 => ContextFailureKind::Permanent,
                401 | 407 | 511 => ContextFailureKind::SessionInvalid,
                408 | 429 | 500..=599 => ContextFailureKind::Transient,
                _ => ContextFailureKind::Transient,
            };
        }

        match error.kind {
            ErrorKind::InvalidArgument | ErrorKind::NotFound => ContextFailureKind::Permanent,
            ErrorKind::Unauthenticated => ContextFailureKind::SessionInvalid,
            _ => ContextFailureKind::Transient,
        }
    }

    pub fn mark_next_unavailable(&mut self, kind: ContextFailureKind) {
        if let Some((next, _, _)) = self.find_next() {
            let retry_after = match kind {
                ContextFailureKind::Permanent => RETRY_UNAVAILABLE,
                ContextFailureKind::Transient => RETRY_TRANSIENT,
                ContextFailureKind::SessionInvalid => RETRY_SESSION,
            };
            self.unavailable_contexts.insert(
                next.clone(),
                ContextFailure {
                    last_try: Instant::now(),
                    retry_after,
                },
            );
        }
    }

    pub fn mark_next_resolved(&mut self) {
        if let Some((next, _, _)) = self.find_next() {
            let next = next.clone();
            self.unavailable_contexts.remove(&next);
        }
    }

    pub fn apply_next_context(
        &self,
        state: &mut ConnectState,
        mut context: Context,
    ) -> Result<Option<Vec<ResolveContext>>, Error> {
        let (next, _, _) = self.find_next().ok_or(ContextResolverError::NoNext)?;

        let remaining = match next.action {
            ContextAction::Append if context.pages.len() == 1 => state
                .fill_context_from_page(context.pages.remove(0))
                .map(|_| None),
            ContextAction::Replace => {
                let remaining = state.update_context(context, next.update);
                if let Resolve::Context(ref ctx) = next.resolve {
                    state.merge_context(ctx.pages.clone().pop());
                }

                remaining
            }
            ContextAction::Append => {
                warn!("unexpected page size: {context:#?}");
                Err(ContextResolverError::UnexpectedPagesSize(context.pages.len()).into())
            }
        }?;

        Ok(remaining.map(|remaining| {
            remaining
                .into_iter()
                .map(ResolveContext::append_context)
                .collect::<Vec<_>>()
        }))
    }

    pub fn try_finish(
        &self,
        state: &mut ConnectState,
        transfer_state: &mut Option<TransferState>,
    ) -> bool {
        let (next, _, _) = match self.find_next() {
            None => return false,
            Some(next) => next,
        };

        // when there is only one update type, we are the last of our kind, so we should update the state
        if self
            .queue
            .iter()
            .filter(|resolve| resolve.update == next.update)
            .count()
            != 1
        {
            return false;
        }

        match (next.update, state.active_context) {
            (ContextType::Default, ContextType::Default) | (ContextType::Autoplay, _) => {
                debug!(
                    "last item of type <{:?}>, finishing state setup",
                    next.update
                );
            }
            (ContextType::Default, _) => {
                debug!("skipped finishing default, because it isn't the active context");
                return false;
            }
        }

        let active_ctx = state.get_context(state.active_context);
        let res = if let Some(transfer_state) = transfer_state.take() {
            state.finish_transfer(transfer_state)
        } else if state.shuffling_context() && next.update == ContextType::Default {
            state.shuffle_new()
        } else if matches!(active_ctx, Ok(ctx) if ctx.index.track == 0) {
            // has context, and context is not touched
            // when the index is not zero, the next index was already evaluated elsewhere
            let ctx = active_ctx.expect("checked by precondition");
            let idx = ConnectState::find_index_in_context(ctx, |t| {
                state.current_track(|c| t.uri == c.uri)
            })
            .ok();

            state.reset_playback_to_position(idx)
        } else {
            state.fill_up_next_tracks()
        };

        if let Err(why) = res {
            error!("setup of state failed: {why}, last used resolve {next:#?}")
        }

        state.update_restrictions();
        state.update_queue_revision();

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{config::SessionConfig, http_client::HttpClientError};

    fn resolver(runtime: &tokio::runtime::Runtime) -> ContextResolver {
        let _guard = runtime.enter();
        ContextResolver::new(Session::new(SessionConfig::default(), None))
    }

    fn request() -> ResolveContext {
        ResolveContext::from_uri(
            "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
            "",
            ContextType::Default,
            ContextAction::Replace,
        )
    }

    #[test]
    fn transient_context_failure_remains_retryable_and_queued() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let mut resolver = resolver(&runtime);
        resolver.add(request());
        let error = Error::deadline_exceeded("scripted timeout");
        let kind = resolver.classify_failure(&error);

        resolver.mark_next_unavailable(kind);

        assert_eq!(kind, ContextFailureKind::Transient);
        assert_eq!(resolver.queue.len(), 1);
        assert!(resolver.has_next());
    }

    #[test]
    fn spotify_503_is_transient_not_permanent_context_failure() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let resolver = resolver(&runtime);
        let error: Error =
            HttpClientError::StatusCode("503".parse().expect("valid HTTP status")).into();

        assert_eq!(
            resolver.classify_failure(&error),
            ContextFailureKind::Transient
        );
    }

    #[test]
    fn elapsed_backoff_uses_now_minus_last_try() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let mut resolver = resolver(&runtime);
        let request = request();
        resolver.unavailable_contexts.insert(
            request.clone(),
            ContextFailure {
                last_try: Instant::now() - RETRY_UNAVAILABLE - Duration::from_secs(1),
                retry_after: RETRY_UNAVAILABLE,
            },
        );

        resolver.add(request.clone());

        assert!(resolver.queue.contains(&request));
        assert!(!resolver.unavailable_contexts.contains_key(&request));
    }

    #[test]
    fn invalid_session_is_classified_separately() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let resolver = resolver(&runtime);
        resolver.session.shutdown();

        assert_eq!(
            resolver.classify_failure(&Error::unavailable("dead AP")),
            ContextFailureKind::SessionInvalid
        );
        assert!(!resolver.has_next());
    }

    #[test]
    fn replacing_session_preserves_pending_context_and_backoff() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let mut resolver = resolver(&runtime);
        let request = request();
        resolver.add(request.clone());
        resolver.mark_next_unavailable(ContextFailureKind::Transient);
        let replacement = {
            let _guard = runtime.enter();
            Session::new(SessionConfig::default(), None)
        };
        let replacement_id = replacement.session_id();

        resolver.set_session(replacement);

        assert_eq!(resolver.session.session_id(), replacement_id);
        assert!(resolver.queue.contains(&request));
        assert!(resolver.unavailable_contexts.contains_key(&request));
    }
}
