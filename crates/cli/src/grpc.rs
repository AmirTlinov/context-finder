use crate::cache::CacheConfig;
use crate::command::infra::HealthPort;
use crate::command::{classify_error, CommandRequest};
use tonic::{Request, Response, Status};

pub mod proto {
    tonic::include_proto!("command");
}

use proto::command_service_server::{CommandService, CommandServiceServer};
use proto::{CommandPayload, CommandReply, HealthReply, HealthRequest};

#[derive(Clone)]
pub struct GrpcServer {
    cache: CacheConfig,
}

impl GrpcServer {
    pub fn new(cache: CacheConfig) -> Self {
        Self { cache }
    }

    pub fn into_server(self) -> CommandServiceServer<Self> {
        CommandServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl CommandService for GrpcServer {
    async fn execute(
        &self,
        request: Request<CommandPayload>,
    ) -> Result<Response<CommandReply>, Status> {
        let json = request.into_inner().json;
        let req: CommandRequest = serde_json::from_slice(&json)
            .map_err(|e| Status::invalid_argument(format!("invalid json: {e}")))?;

        let resp = match crate::command::execute(req, self.cache.clone()).await {
            Ok(r) => r,
            Err(err) => {
                let message = format!("{err:#}");
                let hints = classify_error(&message);
                crate::command::CommandResponse::error_with_hints(message, hints)
            }
        };

        let bytes = serde_json::to_vec(&resp)
            .map_err(|e| Status::internal(format!("serialize failed: {e}")))?;
        Ok(Response::new(CommandReply { json: bytes }))
    }

    async fn health(
        &self,
        request: Request<HealthRequest>,
    ) -> Result<Response<HealthReply>, Status> {
        let project = request.into_inner().project;
        let root = if project.is_empty() {
            std::env::current_dir()
        } else {
            Ok(std::path::PathBuf::from(project))
        }
        .map_err(|e| Status::internal(format!("resolve project: {e}")))?;

        let report = HealthPort
            .probe(&root)
            .await
            .map_err(|e| Status::internal(format!("health probe failed: {e}")))?;

        Ok(Response::new(HealthReply {
            status: report.status,
            last_success_unix_ms: report.last_success_unix_ms.unwrap_or(0),
            last_failure_unix_ms: report.last_failure_unix_ms.unwrap_or(0),
            failures: report.failures,
            index_size_bytes: report.index_size_bytes.unwrap_or(0),
            graph_cache_size_bytes: report.graph_cache_size_bytes.unwrap_or(0),
            last_failure_reason: report.last_failure_reason.unwrap_or_default(),
        }))
    }
}
