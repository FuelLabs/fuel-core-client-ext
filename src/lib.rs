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
        tx::TransactionStatus,
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
    pub status: Option<TransactionStatus>,
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
    use serde::Serialize;
    use csv::Writer;
    use fuel_core_types::fuel_crypto;
    use fuel_core_types::fuel_crypto::coins_bip32::ecdsa::{RecoveryId, VerifyingKey};
    use fuel_core_types::fuel_crypto::coins_bip32::prelude::k256;
    
    
    
    
    
    use super::*;

    #[derive(Serialize)]
    struct ProducerRecord {
        block_number: u64,
        producer: String,
        correct_public_key: String,
        rec2: String,
    }

    #[tokio::test]
    async fn testnet_works() {
        let client = FuelClient::new("http://127.0.0.1:4000")
            .expect("Should connect to the beta 5 network");

        let from_block = 9704468;
        let to_block = 9704580; // stopped at

        let file_path = "producers.csv";
        let mut wtr = Writer::from_path(file_path).expect("Couldn't create CSV writer");

        for block in from_block..to_block {
            let fetched_block = client.block_by_height(block.into()).await.unwrap().unwrap();
            let public_key = fetched_block.block_producer.unwrap();

            let sig = match fetched_block.consensus {
                fuel_core_client::client::types::Consensus::PoAConsensus(poa) => {
                    k256::ecdsa::Signature::from_bytes(poa.signature.as_slice().into()).unwrap()
                }
                _ => panic!("bad")
            };

            let rec_id = RecoveryId::new(false, false);

            let rec_id_2 = RecoveryId::new(true, false);

            let message = fuel_crypto::Message::from_bytes(*fetched_block.id);

            let rec1 = VerifyingKey::recover_from_prehash(&*message, &sig.into(), rec_id).unwrap();
            let rec1_as_pubkey = PublicKey::from(&rec1);
            let rec2 = VerifyingKey::recover_from_prehash(&*message, &sig.into(), rec_id_2).unwrap();
            let rec2_as_pubkey = PublicKey::from(&rec2);

            let record = ProducerRecord {
                block_number: block as u64,
                producer: public_key.to_string(),
                correct_public_key: rec1_as_pubkey.to_string(),
                rec2: rec2_as_pubkey.to_string(),
            };

            wtr.serialize(&record).expect("Couldn't write record to CSV");
            wtr.flush().expect("Couldn't flush CSV writer");
        }

        println!("Producers (with correct public key) written to {}", file_path);
    }
}