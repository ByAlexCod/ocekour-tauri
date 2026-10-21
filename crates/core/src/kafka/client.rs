//! Client Kafka minimal (protocole natif) : lecture **sans groupe de consommateurs ni commit**
//! (Metadata, ListOffsets, Fetch), SASL PLAIN / SCRAM-SHA-256 / SCRAM-SHA-512, TLS. Produce,
//! CreateTopics et ListGroups servent aux tests.

use crate::config::KafkaConnection;
use crate::error::{ErrorCode, SourceError};
use crate::net;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use kafka_protocol::messages::*;
use kafka_protocol::messages::create_topics_request::CreatableTopic;
use kafka_protocol::messages::fetch_request::{FetchPartition, FetchTopic};
use kafka_protocol::messages::list_offsets_request::{ListOffsetsPartition, ListOffsetsTopic};
use kafka_protocol::messages::metadata_request::MetadataRequestTopic;
use kafka_protocol::messages::produce_request::{PartitionProduceData, TopicProduceData};
use kafka_protocol::protocol::{Decodable, Encodable, Request, StrBytes};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

const CLIENT_ID: &str = "ocekour";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// Erreur Kafka (réseau, protocole, authentification) avant traduction.
#[derive(Debug)]
pub enum KafkaError {
    Io(std::io::Error),
    Tls(SourceError),
    Auth(String),
    /// Code d'erreur du protocole (et message éventuel du broker).
    Code(i16, String),
    Protocol(String),
    Closed,
    Timeout(String),
}

impl std::fmt::Display for KafkaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KafkaError::Io(e) => write!(f, "{e}"),
            KafkaError::Tls(e) => write!(f, "{e}"),
            KafkaError::Auth(m) => write!(f, "{m}"),
            KafkaError::Code(c, m) => write!(f, "{} ({c}){}", error_name(*c), if m.is_empty() { String::new() } else { format!(" : {m}") }),
            KafkaError::Protocol(m) => write!(f, "{m}"),
            KafkaError::Closed => write!(f, "Connection closed"),
            KafkaError::Timeout(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for KafkaError {}

impl From<std::io::Error> for KafkaError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::UnexpectedEof { KafkaError::Closed } else { KafkaError::Io(e) }
    }
}

/// Nom lisible des codes d'erreur courants.
pub fn error_name(code: i16) -> &'static str {
    match code {
        1 => "OFFSET_OUT_OF_RANGE",
        3 => "UNKNOWN_TOPIC_OR_PARTITION",
        5 => "LEADER_NOT_AVAILABLE",
        6 => "NOT_LEADER_OR_FOLLOWER",
        7 => "REQUEST_TIMED_OUT",
        29 => "TOPIC_AUTHORIZATION_FAILED",
        30 => "GROUP_AUTHORIZATION_FAILED",
        31 => "CLUSTER_AUTHORIZATION_FAILED",
        33 => "UNSUPPORTED_SASL_MECHANISM",
        34 => "ILLEGAL_SASL_STATE",
        35 => "UNSUPPORTED_VERSION",
        36 => "TOPIC_ALREADY_EXISTS",
        58 => "SASL_AUTHENTICATION_FAILED",
        100 => "UNKNOWN_TOPIC_ID",
        _ => "KAFKA_ERROR",
    }
}

/// Erreur Kafka → message actionnable.
pub fn to_source_error(e: &KafkaError, brokers: &[String]) -> SourceError {
    match e {
        KafkaError::Auth(_) | KafkaError::Code(58, _) => SourceError::new(
            ErrorCode::Unauthorized,
            "Authentification Kafka refusée : vérifiez l’utilisateur, le mot de passe et le mécanisme (SCRAM-SHA-512…).",
        ),
        KafkaError::Code(29..=31, _) => {
            SourceError::new(ErrorCode::Forbidden, "Accès refusé par Kafka (ACL) : droits insuffisants sur ce topic.")
        }
        KafkaError::Code(33, _) => SourceError::new(
            ErrorCode::Unauthorized,
            "Mécanisme SASL refusé par le broker : vérifiez le mécanisme (SCRAM-SHA-512, SCRAM-SHA-256, PLAIN).",
        ),
        KafkaError::Tls(se) => se.clone(),
        KafkaError::Io(io) => net::classify(io, &format!("kafka://{}", brokers.first().cloned().unwrap_or_default())),
        KafkaError::Closed => SourceError::new(
            ErrorCode::Network,
            "Connexion fermée par le broker : vérifiez le protocole de sécurité (SASL_PLAINTEXT, SASL_SSL, PLAINTEXT).",
        ),
        KafkaError::Timeout(m) => SourceError::new(ErrorCode::Timeout, m.clone()),
        other => SourceError::new(ErrorCode::Network, format!("Kafka : {other}")),
    }
}

pub type Result<T> = std::result::Result<T, KafkaError>;

/// Une connexion à un broker.
pub struct Conn {
    io: Box<dyn Io>,
    correlation: i32,
    versions: HashMap<i16, (i16, i16)>,
}

impl Conn {
    /// Version à utiliser : la plus haute commune, ou erreur.
    pub fn version<R: Request>(&self, cap: i16) -> Result<i16> {
        let (bmin, bmax) = self.versions.get(&R::KEY).copied().unwrap_or((0, -1));
        let max = bmax.min(R::VERSIONS.max).min(cap);
        let min = bmin.max(R::VERSIONS.min);
        if max < min {
            return Err(KafkaError::Protocol(format!("API {} non prise en charge par le broker (versions {bmin}..{bmax})", R::KEY)));
        }
        Ok(max)
    }

    pub async fn send<R: Request>(&mut self, request: &R, version: i16) -> Result<R::Response> {
        tokio::time::timeout(REQUEST_TIMEOUT, self.exchange(request, version))
            .await
            .map_err(|_| KafkaError::Timeout("Kafka : pas de réponse du broker (délai dépassé).".into()))?
    }

    async fn exchange<R: Request>(&mut self, request: &R, version: i16) -> Result<R::Response> {
        self.correlation += 1;
        let api_key = ApiKey::try_from(R::KEY).map_err(|_| KafkaError::Protocol("API inconnue".into()))?;
        let header = RequestHeader::default()
            .with_request_api_key(R::KEY)
            .with_request_api_version(version)
            .with_correlation_id(self.correlation)
            .with_client_id(Some(StrBytes::from_static_str(CLIENT_ID)));
        let mut body = BytesMut::new();
        header.encode(&mut body, api_key.request_header_version(version)).map_err(|e| KafkaError::Protocol(e.to_string()))?;
        request.encode(&mut body, version).map_err(|e| KafkaError::Protocol(e.to_string()))?;
        let mut frame = BytesMut::with_capacity(body.len() + 4);
        frame.put_i32(body.len() as i32);
        frame.extend_from_slice(&body);
        self.io.write_all(&frame).await?;
        self.io.flush().await?;
        let len = self.io.read_i32().await?;
        if !(0..=256 * 1024 * 1024).contains(&len) { return Err(KafkaError::Protocol(format!("taille de réponse invalide ({len})"))); }
        let mut buf = vec![0u8; len as usize];
        self.io.read_exact(&mut buf).await?;
        let mut buf = Bytes::from(buf);
        let response_header = ResponseHeader::decode(&mut buf, api_key.response_header_version(version)).map_err(|e| KafkaError::Protocol(e.to_string()))?;
        if response_header.correlation_id != self.correlation {
            return Err(KafkaError::Protocol("réponse inattendue (corrélation)".into()));
        }
        R::Response::decode(&mut buf, version).map_err(|e| KafkaError::Protocol(e.to_string()))
    }
}

/// Paramètres de connexion d'une connexion Kafka.
#[derive(Clone)]
pub struct Options {
    pub brokers: Vec<String>,
    pub protocol: String,
    pub mechanism: String,
    pub username: String,
    pub password: String,
    pub tls: crate::config::Tls,
}

impl Options {
    pub fn of(connection: &KafkaConnection, secret: &str) -> Self {
        Options {
            brokers: connection.brokers.clone(),
            protocol: connection.security.protocol.clone(),
            mechanism: connection.security.mechanism.clone(),
            username: connection.security.username.clone().unwrap_or_default(),
            password: secret.to_string(),
            tls: connection.tls.clone(),
        }
    }
}

async fn open(address: &str, opts: &Options) -> Result<Conn> {
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address))
        .await
        .map_err(|_| KafkaError::Timeout(format!("Connexion à {address} : délai dépassé.")))??;
    let _ = tcp.set_nodelay(true);
    let io: Box<dyn Io> = if opts.protocol == "SASL_SSL" {
        let config = net::rustls_config(&opts.tls).map_err(KafkaError::Tls)?;
        let host = address.rsplit_once(':').map(|(h, _)| h).unwrap_or(address).trim_matches(['[', ']']);
        let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| KafkaError::Protocol(e.to_string()))?;
        let tls = tokio::time::timeout(CONNECT_TIMEOUT, tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, tcp))
            .await
            .map_err(|_| KafkaError::Timeout(format!("Connexion TLS à {address} : délai dépassé.")))?
            .map_err(|e| KafkaError::Tls(net::classify(&e, &format!("kafka://{address}"))))?;
        Box::new(tls)
    } else {
        Box::new(tcp)
    };
    let mut conn = Conn { io, correlation: 0, versions: HashMap::new() };
    // Versions des API du broker (v0 : comprise par tous).
    let versions = conn.send(&ApiVersionsRequest::default(), 0).await?;
    if versions.error_code != 0 { return Err(KafkaError::Code(versions.error_code, String::new())); }
    conn.versions = versions.api_keys.iter().map(|k| (k.api_key, (k.min_version, k.max_version))).collect();
    if opts.protocol != "PLAINTEXT" { authenticate(&mut conn, opts).await?; }
    Ok(conn)
}

async fn authenticate(conn: &mut Conn, opts: &Options) -> Result<()> {
    let handshake = SaslHandshakeRequest::default().with_mechanism(StrBytes::from_string(opts.mechanism.clone()));
    let v = conn.version::<SaslHandshakeRequest>(1)?;
    let r = conn.send(&handshake, v).await?;
    if r.error_code != 0 { return Err(KafkaError::Code(r.error_code, String::new())); }
    let v = conn.version::<SaslAuthenticateRequest>(2)?;
    let step = |bytes: Vec<u8>| SaslAuthenticateRequest::default().with_auth_bytes(Bytes::from(bytes));
    let fail = |r: &SaslAuthenticateResponse| {
        KafkaError::Auth(r.error_message.as_ref().map(|m| m.to_string()).unwrap_or_else(|| "authentification refusée".into()))
    };
    if opts.mechanism == "PLAIN" {
        let msg = format!("\0{}\0{}", opts.username, opts.password).into_bytes();
        let r = conn.send(&step(msg), v).await?;
        if r.error_code != 0 { return Err(fail(&r)); }
        return Ok(());
    }
    let mut scram = super::scram::Scram::new(&opts.mechanism, &opts.username, &opts.password);
    let r = conn.send(&step(scram.client_first().into_bytes()), v).await?;
    if r.error_code != 0 { return Err(fail(&r)); }
    let final_msg = scram.client_final(&String::from_utf8_lossy(&r.auth_bytes)).map_err(KafkaError::Auth)?;
    let r = conn.send(&step(final_msg.into_bytes()), v).await?;
    if r.error_code != 0 { return Err(fail(&r)); }
    scram.verify_server(&String::from_utf8_lossy(&r.auth_bytes)).map_err(KafkaError::Auth)?;
    Ok(())
}

/// Description d'un topic.
#[derive(Debug, Clone)]
pub struct TopicInfo {
    pub name: String,
    pub id: uuid::Uuid,
    pub internal: bool,
    pub error: i16,
    /// (index, leader).
    pub partitions: Vec<(i32, i32)>,
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub brokers: HashMap<i32, String>,
    pub topics: Vec<TopicInfo>,
}

/// Données d'une partition lues par Fetch.
#[derive(Debug, Clone)]
pub struct PartitionFetch {
    pub partition: i32,
    pub error: i16,
    pub log_start_offset: i64,
    pub records: Option<Bytes>,
}

/// Client : connexions par broker, réutilisées.
pub struct Client {
    pub opts: Options,
    conns: Mutex<HashMap<String, Arc<Mutex<Conn>>>>,
    brokers: Mutex<HashMap<i32, String>>,
}

impl Client {
    pub fn new(opts: Options) -> Self {
        Client { opts, conns: Mutex::new(HashMap::new()), brokers: Mutex::new(HashMap::new()) }
    }

    async fn conn_to(&self, address: &str) -> Result<Arc<Mutex<Conn>>> {
        if let Some(c) = self.conns.lock().await.get(address) { return Ok(c.clone()); }
        let conn = Arc::new(Mutex::new(open(address, &self.opts).await?));
        self.conns.lock().await.insert(address.to_string(), conn.clone());
        Ok(conn)
    }

    /// Connexion au premier broker joignable de la liste d'amorçage.
    async fn any(&self) -> Result<Arc<Mutex<Conn>>> {
        let mut last = None;
        for b in &self.opts.brokers {
            match self.conn_to(b).await {
                Ok(c) => return Ok(c),
                Err(e @ (KafkaError::Auth(_) | KafkaError::Code(..) | KafkaError::Tls(_))) => return Err(e),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| KafkaError::Protocol("aucun broker".into())))
    }

    async fn node(&self, id: i32) -> Result<Arc<Mutex<Conn>>> {
        let address = self.brokers.lock().await.get(&id).cloned();
        match address {
            Some(a) => self.conn_to(&a).await,
            None => {
                self.metadata(Some(&[])).await?;
                let a = self.brokers.lock().await.get(&id).cloned().ok_or_else(|| KafkaError::Protocol(format!("broker {id} inconnu")))?;
                self.conn_to(&a).await
            }
        }
    }

    /// Ferme les connexions (elles seront rouvertes au besoin).
    pub async fn reset(&self) { self.conns.lock().await.clear(); }

    /// Métadonnées : `None` = tous les topics.
    pub async fn metadata(&self, topics: Option<&[String]>) -> Result<Metadata> {
        let conn = self.any().await?;
        let mut conn = conn.lock().await;
        let v = conn.version::<MetadataRequest>(12)?;
        let mut req = MetadataRequest::default();
        req.topics = topics.map(|t| t.iter().map(|n| MetadataRequestTopic::default().with_name(Some(TopicName(StrBytes::from_string(n.clone()))))).collect());
        req.allow_auto_topic_creation = false;
        let r = conn.send(&req, v).await?;
        drop(conn);
        let brokers: HashMap<i32, String> = r.brokers.iter().map(|b| (b.node_id.0, format!("{}:{}", b.host, b.port))).collect();
        self.brokers.lock().await.extend(brokers.clone());
        Ok(Metadata {
            brokers,
            topics: r
                .topics
                .iter()
                .map(|t| TopicInfo {
                    name: t.name.as_ref().map(|n| n.0.to_string()).unwrap_or_default(),
                    id: t.topic_id,
                    internal: t.is_internal,
                    error: t.error_code,
                    partitions: t.partitions.iter().map(|p| (p.partition_index, p.leader_id.0)).collect(),
                })
                .collect(),
        })
    }

    /// Offsets à un horodatage (`-1` fin, `-2` début) par partition : (partition, offset).
    pub async fn list_offsets(&self, topic: &str, partitions: &[(i32, i32)], timestamp: i64) -> Result<HashMap<i32, i64>> {
        let mut by_leader: HashMap<i32, Vec<i32>> = HashMap::new();
        for (p, leader) in partitions { by_leader.entry(*leader).or_default().push(*p); }
        let mut out = HashMap::new();
        for (leader, parts) in by_leader {
            let conn = self.node(leader).await?;
            let mut conn = conn.lock().await;
            let v = conn.version::<ListOffsetsRequest>(7)?;
            let mut req = ListOffsetsRequest::default();
            req.replica_id = BrokerId(-1);
            req.topics = vec![ListOffsetsTopic::default()
                .with_name(TopicName(StrBytes::from_string(topic.to_string())))
                .with_partitions(parts.iter().map(|p| ListOffsetsPartition::default().with_partition_index(*p).with_current_leader_epoch(-1).with_timestamp(timestamp)).collect())];
            let r = conn.send(&req, v).await?;
            for t in r.topics {
                for p in t.partitions {
                    if p.error_code != 0 { return Err(KafkaError::Code(p.error_code, String::new())); }
                    out.insert(p.partition_index, p.offset);
                }
            }
        }
        Ok(out)
    }

    /// Fetch sur un broker leader. `topic_id` sert à partir de Fetch v13 (Kafka ≥ 3.1).
    pub async fn fetch(&self, leader: i32, topic: &str, topic_id: uuid::Uuid, partitions: &[(i32, i64)], max_wait_ms: i32) -> Result<Vec<PartitionFetch>> {
        const PARTITION_MAX_BYTES: i32 = 1_048_576;
        let conn = self.node(leader).await?;
        let mut conn = conn.lock().await;
        let mut v = conn.version::<FetchRequest>(17)?;
        if topic_id.is_nil() && v >= 13 { v = 12; }
        let mut ft = FetchTopic::default().with_partitions(
            partitions
                .iter()
                .map(|(p, offset)| {
                    FetchPartition::default()
                        .with_partition(*p)
                        .with_fetch_offset(*offset)
                        .with_current_leader_epoch(-1)
                        .with_last_fetched_epoch(-1)
                        .with_log_start_offset(-1)
                        .with_partition_max_bytes(PARTITION_MAX_BYTES)
                })
                .collect(),
        );
        if v >= 13 { ft.topic_id = topic_id } else { ft.topic = TopicName(StrBytes::from_string(topic.to_string())) }
        let mut req = FetchRequest::default();
        req.replica_id = BrokerId(-1);
        req.max_wait_ms = max_wait_ms;
        req.min_bytes = 1;
        req.max_bytes = 16 * PARTITION_MAX_BYTES;
        req.isolation_level = 0;
        req.session_id = 0;
        req.session_epoch = -1;
        req.topics = vec![ft];
        // L'attente côté broker s'ajoute au délai de la requête.
        let r = tokio::time::timeout(REQUEST_TIMEOUT + Duration::from_millis(max_wait_ms as u64), conn.exchange(&req, v))
            .await
            .map_err(|_| KafkaError::Timeout("Kafka : pas de réponse du broker (délai dépassé).".into()))??;
        if r.error_code != 0 { return Err(KafkaError::Code(r.error_code, String::new())); }
        Ok(r.responses
            .into_iter()
            .flat_map(|t| t.partitions)
            .map(|p| PartitionFetch { partition: p.partition_index, error: p.error_code, log_start_offset: p.log_start_offset, records: p.records })
            .collect())
    }

    /// Crée des topics (tests).
    pub async fn create_topics(&self, names: &[&str], partitions: i32, replicas: i16) -> Result<()> {
        let conn = self.any().await?;
        let mut conn = conn.lock().await;
        let v = conn.version::<CreateTopicsRequest>(5)?;
        let mut req = CreateTopicsRequest::default();
        req.timeout_ms = 30_000;
        req.topics = names
            .iter()
            .map(|n| CreatableTopic::default().with_name(TopicName(StrBytes::from_string(n.to_string()))).with_num_partitions(partitions).with_replication_factor(replicas))
            .collect();
        let r = conn.send(&req, v).await?;
        for t in r.topics {
            if t.error_code != 0 && t.error_code != 36 { return Err(KafkaError::Code(t.error_code, t.error_message.map(|m| m.to_string()).unwrap_or_default())); }
        }
        Ok(())
    }

    /// Écrit des messages (tests) : (partition, horodatage, clé, valeur, headers).
    pub async fn produce(&self, topic: &str, messages: Vec<ProduceMessage>) -> Result<()> {
        let meta = self.metadata(Some(&[topic.to_string()])).await?;
        let info = meta.topics.into_iter().next().ok_or_else(|| KafkaError::Protocol("topic absent".into()))?;
        let mut by_partition: HashMap<i32, Vec<ProduceMessage>> = HashMap::new();
        for m in messages { by_partition.entry(m.partition).or_default().push(m); }
        for (partition, msgs) in by_partition {
            let leader = info.partitions.iter().find(|(p, _)| *p == partition).map(|(_, l)| *l).unwrap_or(0);
            let records: Vec<kafka_protocol::records::Record> = msgs
                .iter()
                .enumerate()
                .map(|(i, m)| kafka_protocol::records::Record {
                    transactional: false,
                    control: false,
                    delete_horizon: false,
                    partition_leader_epoch: -1,
                    producer_id: -1,
                    producer_epoch: -1,
                    timestamp_type: kafka_protocol::records::TimestampType::Creation,
                    offset: i as i64,
                    // Un seul lot : (offset − séquence) constant, séquence de base −1.
                    sequence: i as i32 - 1,
                    timestamp: m.timestamp.unwrap_or_else(crate::text::now_ms),
                    key: m.key.clone().map(Bytes::from),
                    value: m.value.clone().map(Bytes::from),
                    headers: m.headers.iter().map(|(k, v)| (StrBytes::from_string(k.clone()), Some(Bytes::from(v.clone())))).collect(),
                })
                .collect();
            let mut buf = BytesMut::new();
            kafka_protocol::records::RecordBatchEncoder::encode(
                &mut buf,
                &records,
                &kafka_protocol::records::RecordEncodeOptions { version: 2, compression: kafka_protocol::records::Compression::None },
            )
            .map_err(|e| KafkaError::Protocol(e.to_string()))?;
            let conn = self.node(leader).await?;
            let mut conn = conn.lock().await;
            let v = conn.version::<ProduceRequest>(9)?;
            let mut req = ProduceRequest::default();
            req.acks = -1;
            req.timeout_ms = 30_000;
            req.topic_data = vec![TopicProduceData::default()
                .with_name(TopicName(StrBytes::from_string(topic.to_string())))
                .with_topic_id(info.id)
                .with_partition_data(vec![PartitionProduceData::default().with_index(partition).with_records(Some(buf.freeze()))])];
            let r = conn.send(&req, v).await?;
            for t in r.responses {
                for p in t.partition_responses {
                    if p.error_code != 0 { return Err(KafkaError::Code(p.error_code, String::new())); }
                }
            }
        }
        Ok(())
    }

    /// Groupes de consommateurs (tests : vérifier qu'aucun n'est créé).
    pub async fn list_groups(&self) -> Result<Vec<String>> {
        let conn = self.any().await?;
        let mut conn = conn.lock().await;
        let v = conn.version::<ListGroupsRequest>(4)?;
        let r = conn.send(&ListGroupsRequest::default(), v).await?;
        Ok(r.groups.into_iter().map(|g| g.group_id.0.to_string()).collect())
    }
}

/// Message à écrire (tests).
#[derive(Debug, Clone, Default)]
pub struct ProduceMessage {
    pub partition: i32,
    pub timestamp: Option<i64>,
    pub key: Option<Vec<u8>>,
    pub value: Option<Vec<u8>>,
    pub headers: Vec<(String, Vec<u8>)>,
}

/// Message lu.
#[derive(Debug, Clone)]
pub struct RawMessage {
    pub partition: i32,
    pub offset: i64,
    pub timestamp: i64,
    pub key: Option<Bytes>,
    pub value: Option<Bytes>,
    pub headers: Vec<(String, Option<Bytes>)>,
}

/// Lots d'enregistrements d'une partition ; renvoie les messages et le prochain offset (lots
/// tronqués en fin de réponse ignorés).
pub fn decode_records(partition: i32, mut data: Bytes, from: i64) -> (Vec<RawMessage>, Option<i64>) {
    let mut out = vec![];
    let mut next: Option<i64> = None;
    while data.remaining() >= 61 {
        let base_offset = i64::from_be_bytes(data[0..8].try_into().unwrap());
        let batch_len = i32::from_be_bytes(data[8..12].try_into().unwrap());
        let total = 12 + batch_len.max(0) as usize;
        if data.remaining() < total { break; }
        let magic = data[16] as i8;
        let last_delta = i32::from_be_bytes(data[23..27].try_into().unwrap());
        let mut batch = data.split_to(total);
        let last = base_offset + last_delta as i64;
        if magic == 2 {
            if let Ok(set) = kafka_protocol::records::RecordBatchDecoder::decode(&mut batch) {
                for r in set.records {
                    if r.control || r.offset < from { continue; }
                    out.push(RawMessage {
                        partition,
                        offset: r.offset,
                        timestamp: r.timestamp,
                        key: r.key,
                        value: r.value,
                        headers: r.headers.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
                    });
                }
            }
        }
        // Lot sans enregistrement lisible (marqueurs de transaction…) : on avance quand même.
        if last >= from { next = Some(next.map_or(last + 1, |n: i64| n.max(last + 1))); }
    }
    (out, next)
}
