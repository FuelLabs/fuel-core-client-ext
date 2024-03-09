use cynic::QueryBuilder;
use fuel_core_client::client::{
    pagination::{
        PaginatedResult,
        PaginationRequest,
    },
    schema::{
        block::{
            BlockByHeightArgs,
            Consensus,
            Header,
        },
        primitives::TransactionId,
        schema,
        tx::transparent_receipt::Receipt,
        BlockId,
        ConnectionArgs,
        HexString,
        PageInfo,
    },
    FuelClient,
};
use fuel_core_types::fuel_crypto::PublicKey;

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./target/schema.sdl",
    graphql_type = "Query",
    variables = "ConnectionArgs"
)]
pub struct FullBlocksQuery {
    #[arguments(after: $after, before: $before, first: $first, last: $last)]
    pub blocks: FullBlockConnection,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./target/schema.sdl", graphql_type = "BlockConnection")]
pub struct FullBlockConnection {
    pub edges: Vec<FullBlockEdge>,
    pub page_info: PageInfo,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./target/schema.sdl", graphql_type = "BlockEdge")]
pub struct FullBlockEdge {
    pub cursor: String,
    pub node: FullBlock,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./target/schema.sdl",
    graphql_type = "Query",
    variables = "BlockByHeightArgs"
)]
pub struct FullBlockByHeightQuery {
    #[arguments(height: $height)]
    pub block: Option<FullBlock>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./target/schema.sdl", graphql_type = "Block")]
pub struct FullBlock {
    pub id: BlockId,
    pub header: Header,
    pub consensus: Consensus,
    pub transactions: Vec<OpaqueTransaction>,
}

impl FullBlock {
    /// Returns the block producer public key, if any.
    pub fn block_producer(&self) -> Option<PublicKey> {
        let message = self.header.id.clone().into_message();
        match &self.consensus {
            Consensus::Genesis(_) => Some(Default::default()),
            Consensus::PoAConsensus(poa) => {
                let signature = poa.signature.clone().into_signature();
                let producer_pub_key = signature.recover(&message);
                producer_pub_key.ok()
            }
            Consensus::Unknown => None,
        }
    }
}

impl From<FullBlockConnection> for PaginatedResult<FullBlock, String> {
    fn from(conn: FullBlockConnection) -> Self {
        PaginatedResult {
            cursor: conn.page_info.end_cursor,
            has_next_page: conn.page_info.has_next_page,
            has_previous_page: conn.page_info.has_previous_page,
            results: conn.edges.into_iter().map(|e| e.node).collect(),
        }
    }
}

#[derive(cynic::QueryFragment, Clone, Debug)]
#[cynic(schema_path = "./target/schema.sdl", graphql_type = "Transaction")]
pub struct OpaqueTransaction {
    pub id: TransactionId,
    pub raw_payload: HexString,
    pub receipts: Option<Vec<Receipt>>,
}

#[async_trait::async_trait]
pub trait ClientExt {
    async fn full_blocks(
        &self,
        request: PaginationRequest<String>,
    ) -> std::io::Result<PaginatedResult<FullBlock, String>>;
}

#[async_trait::async_trait]
impl ClientExt for FuelClient {
    async fn full_blocks(
        &self,
        request: PaginationRequest<String>,
    ) -> std::io::Result<PaginatedResult<FullBlock, String>> {
        let query = FullBlocksQuery::build(request.into());
        let blocks = self.query(query).await?.blocks.into();
        Ok(blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuel_core_chain_config::ChainConfig;
    use fuel_core_client::client::{
        pagination::PageDirection,
        types::{
            TransactionResponse,
            TransactionStatus,
        },
    };
    use fuel_core_types::{
        fuel_tx,
        fuel_tx::{
            field::{
                Inputs,
                Outputs,
            },
            input::coin::{
                CoinPredicate,
                CoinSigned,
            },
            Address,
            Input,
            Output,
            Transaction,
            TxId,
            UniqueIdentifier,
            UtxoId,
        },
        fuel_types::{
            AssetId,
            MessageId,
        },
    };
    use std::{
        collections::{
            HashMap,
            HashSet,
            VecDeque,
        },
        sync::Arc,
    };

    #[tokio::test]
    async fn beta5_works() {
        let client = FuelClient::new("https://beta-5.fuel.network")
            .expect("Should connect to the beta 5 network");

        let request = PaginationRequest {
            cursor: None,
            results: 1,
            direction: PageDirection::Backward,
        };
        let full_block = client.full_blocks(request).await;

        assert!(full_block.is_ok(), "{full_block:?}");
    }

    // 1 ETH
    const THRESHOLD: u64 = 10_000_000;

    #[derive(Default)]
    pub struct UtxoStat {
        spent: TxId,
    }

    #[derive(Default)]
    pub struct Stat {
        chain_config: ChainConfig,
        utxo_ids: HashMap<UtxoId, UtxoStat>,
        txs: HashMap<TxId, Arc<Transaction>>,
        addresses_to_check: VecDeque<Address>,
        checked_addresses: HashSet<Address>,
        utxo_id_to_check: VecDeque<UtxoId>,
        checked_utxo_id: HashMap<UtxoId, u64>,
        message_id: Vec<MessageId>,
    }

    impl Stat {
        pub fn new(chain_config: ChainConfig) -> Self {
            let mut _self = Self {
                chain_config,
                ..Default::default()
            };
            // We know the faucet address is not compromised
            _self
                .checked_addresses
                .insert(FAUCET_ADDRESS.parse().unwrap());
            _self
        }

        pub fn check_address(&mut self, owner: Address) {
            if !self.checked_addresses.contains(&owner) {
                self.addresses_to_check.push_back(owner);
                self.checked_addresses.insert(owner);
            }
        }

        pub fn next_address(&mut self) -> Option<Address> {
            self.addresses_to_check.pop_front()
        }

        pub fn check_utxo(&mut self, utxo_id: UtxoId, amount: u64) {
            if !self.checked_utxo_id.contains_key(&utxo_id) {
                self.utxo_id_to_check.push_back(utxo_id);
                self.checked_utxo_id.insert(utxo_id, amount);
            }
        }

        pub fn next_utxo_id(&mut self) -> Option<UtxoId> {
            self.utxo_id_to_check.pop_front()
        }

        pub fn register_tx(&mut self, tx: Arc<Transaction>) -> TxId {
            let id = tx.id(&self.chain_config.consensus_parameters.chain_id);
            self.txs.insert(id, tx);
            id
        }

        pub fn spent_utxo(&mut self, utxo_id: UtxoId, tx_id: TxId) {
            self.utxo_ids.insert(utxo_id, UtxoStat { spent: tx_id });
        }
    }

    pub trait LookingFor {
        fn inputs(&self) -> &[Input];
        fn outputs(&self) -> &[Output];
    }

    impl LookingFor for Transaction {
        fn inputs(&self) -> &[Input] {
            match self {
                Transaction::Script(script) => script.inputs(),
                Transaction::Create(create) => create.inputs(),
                Transaction::Mint(_) => {
                    unreachable!()
                }
            }
        }

        fn outputs(&self) -> &[Output] {
            match self {
                Transaction::Script(script) => script.outputs(),
                Transaction::Create(create) => create.outputs(),
                Transaction::Mint(_) => {
                    unreachable!()
                }
            }
        }
    }

    async fn analyze_transactions(
        base_asset_id: AssetId,
        txs: Vec<TransactionResponse>,
        stat: &mut Stat,
    ) {
        for response in txs {
            let tx = Arc::new(response.transaction);
            let tx_id = stat.register_tx(tx.clone());
            let inputs = tx.inputs();

            let receipts = match response.status {
                TransactionStatus::Success { receipts, .. } => receipts,
                TransactionStatus::Failure { receipts, .. } => receipts,
                _ => unreachable!(),
            };

            for receipt in receipts {
                match receipt {
                    fuel_tx::Receipt::Transfer {
                        asset_id, amount, ..
                    } => {
                        if amount <= THRESHOLD {
                            continue;
                        }

                        if asset_id != base_asset_id {
                            continue;
                        }
                        println!("Found transfer {asset_id} {amount}")
                    }
                    fuel_tx::Receipt::TransferOut {
                        asset_id, amount, ..
                    } => {
                        if amount <= THRESHOLD {
                            continue;
                        }

                        if asset_id != base_asset_id {
                            continue;
                        }
                        println!("Found transfer output {asset_id} {amount}")
                    }
                    fuel_tx::Receipt::MessageOut {
                        amount,
                        nonce,
                        sender,
                        recipient,
                        data,
                        ..
                    } => {
                        if amount <= THRESHOLD {
                            continue;
                        }
                        let message_id = Input::compute_message_id(
                            &sender,
                            &recipient,
                            &nonce,
                            amount,
                            &data.unwrap(),
                        );
                        stat.message_id.push(message_id);
                        println!(
                            "Found message out `{message_id}` with amount `{amount}` ~ {}",
                            amount as f32 / 1e9
                        )
                    }
                    _ => {}
                }
            }

            for input in inputs {
                match input {
                    Input::CoinSigned(CoinSigned { utxo_id, owner, .. })
                    | Input::CoinPredicate(CoinPredicate { utxo_id, owner, .. }) => {
                        stat.spent_utxo(*utxo_id, tx_id);
                        stat.check_address(*owner);
                    }
                    _ => {
                        // Ignore non-coin inputs
                    }
                }
            }
        }
    }

    const FAUCET_ADDRESS: &'static str =
        "3af978e630544c58c62606c077bdf77cc2dd082e483b4bb16a2d08672d2d5a5c";

    #[tokio::test]
    async fn find_addresses() {
        println!("Started find_addresses");

        let chain_config = include_bytes!("../chain_config.json");
        let chain_config: ChainConfig = serde_json::from_slice(chain_config).unwrap();

        let coins = chain_config.initial_state.clone().unwrap().coins.unwrap();
        let base_asset_id = chain_config.consensus_parameters.base_asset_id;

        let mut stat = Stat::new(chain_config);
        for (i, coin) in coins.iter().enumerate() {
            stat.check_utxo(UtxoId::new(Default::default(), i as u8 + 1), coin.amount);
            stat.check_address(coin.owner);
        }

        let client = FuelClient::new("http://127.0.0.1:4000")
            .expect("Should connect to the local network");

        let mut unspent_utxos: Vec<UtxoId> = Default::default();

        while let Some(utxo_id) = stat.next_utxo_id() {
            while let Some(owner) = stat.next_address() {
                let request = PaginationRequest {
                    cursor: None,
                    results: 100_000_000,
                    direction: PageDirection::Forward,
                };
                println!("Looking for transactions for {owner}");
                let transactions = client
                    .transactions_by_owner(&owner, request)
                    .await
                    .expect("Should get transactions")
                    .results;
                println!("Found {} transactions for {owner}", transactions.len());
                analyze_transactions(base_asset_id, transactions, &mut stat).await;
            }

            if let Some(spent) = stat.utxo_ids.get(&utxo_id) {
                let tx_id = spent.spent;
                let tx = stat
                    .txs
                    .get(&tx_id)
                    .expect("We always have the transaction in the cache")
                    .clone();

                let outputs = tx.outputs();

                for (i, output) in outputs.iter().enumerate() {
                    match output {
                        Output::Coin {
                            amount,
                            to,
                            asset_id,
                        }
                        | Output::Change {
                            amount,
                            to,
                            asset_id,
                        }
                        | Output::Variable {
                            amount,
                            to,
                            asset_id,
                        } => {
                            if *amount <= THRESHOLD {
                                continue;
                            }

                            if *asset_id != base_asset_id {
                                continue;
                            }

                            stat.check_address(*to);
                            stat.check_utxo(UtxoId::new(tx_id, i as u8), *amount);
                        }
                        _ => {
                            // Ignore contracts for now
                        }
                    }
                }
            } else {
                unspent_utxos.push(utxo_id);
            }
        }

        let unspent_utxos = unspent_utxos
            .into_iter()
            .map(|utxo_id| {
                let amount = stat.checked_utxo_id.get(&utxo_id).unwrap();
                (utxo_id.to_string(), *amount)
            })
            .collect::<Vec<_>>();

        println!("Unspent utxos {}: {:?}", unspent_utxos.len(), unspent_utxos);
        println!(
            "Bridged messages {}: {:?}",
            stat.message_id.len(),
            stat.message_id
        );
    }
}
