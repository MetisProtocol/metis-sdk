use actix_web::{App, HttpResponse, HttpServer, Responder, post, web};
use alloy_sol_types::sol;
use futures::{FutureExt, TryStreamExt};
use metis_primitives::{Address, address, hex};
use reth::transaction_pool::PoolTransaction;
use reth::transaction_pool::TransactionOrigin;
use reth::transaction_pool::TransactionPool;
use reth_ethereum::{
    exex::{ExExContext, ExExEvent},
    node::{EthereumNode, api::FullNodeComponents},
    rpc::eth::utils::recover_raw_transaction,
};
use reth_primitives::Recovered;
use reth_tracing::tracing::info;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// The AI ExEx id.
pub const AI_EXEX_ID: &str = "ai-exex";
/// The AI ExEx node default port.
pub const DEFAULT_AI_ADDR: &str = "127.0.0.1:8081";
/// The Metric contract Address.
pub const METRIC_CONTRACT_ADDRESS: Address = address!("0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");

sol! {
    mapping(uint256 => uint256) public tokenIdToInferenceCostToken;
    function updateInferenceCostToken(uint256 tokenId, uint256 cost) public;
    function getInferenceCostToken(uint256 tokenId) public view returns (uint256);
    function owner() public view virtual returns (address);
}

#[derive(Deserialize, Serialize, Clone)]
pub struct InferenceRequest {
    transaction: String,
    id: usize,
    tokens_of_inference: usize,
}

/// The ExEx struct, representing the initialization and execution of the ExEx.
pub struct ExEx<Node: FullNodeComponents> {
    /// The ExEx context including the tx pool,
    ctx: ExExContext<Node>,
    /// The receiver of the request body.
    rx: mpsc::Receiver<InferenceRequest>,
}

impl<Node: FullNodeComponents> ExEx<Node> {
    pub fn new(ctx: ExExContext<Node>, rx: mpsc::Receiver<InferenceRequest>) -> Self {
        Self { ctx, rx }
    }

    async fn start(mut self) -> eyre::Result<()> {
        loop {
            // Deal all incoming inference requests
            while let Some(request) = self.rx.recv().await {
                let transaction = request
                    .transaction
                    .strip_prefix("0x")
                    .unwrap_or(request.transaction.as_str());
                let tx = match hex::decode(transaction) {
                    Ok(data) => data,
                    Err(e) => {
                        info!("Invalid transaction data: {}", e);
                        continue;
                    }
                };
                let recovered: Recovered<<<<Node as FullNodeComponents>::Pool as TransactionPool>::Transaction as PoolTransaction>::Pooled> = recover_raw_transaction(&tx)?;
                let pool_transaction =
                    <<Node as FullNodeComponents>::Pool as TransactionPool>::Transaction::from_pooled(
                        recovered,
                    );
                // Submit the transaction to the pool with a `Local` origin
                let hash = self
                    .ctx
                    .pool()
                    .add_transaction(TransactionOrigin::Local, pool_transaction)
                    .await?;
                info!(hash = ?hash, "Received tx hash");
            }

            // Process all new chain state notifications
            if let Some(notification) = self.ctx.notifications.try_next().await? {
                if let Some(committed_chain) = notification.committed_chain() {
                    self.ctx
                        .events
                        .send(ExExEvent::FinishedHeight(committed_chain.tip().num_hash()))?;
                }
            }
        }
    }
}

#[post("/chat/completions")]
async fn chat_completions(
    req_body: web::Json<InferenceRequest>,
    tx: web::Data<mpsc::Sender<InferenceRequest>>,
) -> impl Responder {
    if let Err(e) = tx.send(req_body.0).await {
        return HttpResponse::InternalServerError()
            .body(format!("Failed to send transaction: {}", e));
    }

    HttpResponse::Ok().body("Transaction added to the pool")
}

pub fn run_ai_exex() -> eyre::Result<()> {
    reth::cli::Cli::parse_args().run(|builder, _| async move {
        let (tx, rx) = mpsc::channel(32);
        let handle = builder
            .node(EthereumNode::default())
            .install_exex(AI_EXEX_ID, move |ctx| {
                tokio::task::spawn_blocking(move || {
                    tokio::runtime::Handle::current().block_on(async move {
                        // Start an AI node.
                        HttpServer::new(move || {
                            App::new()
                                .app_data(web::Data::new(tx.clone()))
                                .service(chat_completions)
                        })
                        .bind(DEFAULT_AI_ADDR)?
                        .run()
                        .await?;
                        Ok(ExEx::new(ctx, rx).start())
                    })
                })
                .map(|result| result.map_err(Into::into).and_then(|result| result))
            })
            .launch()
            .await?;
        handle.wait_for_node_exit().await
    })
}
