use base64::Engine;
use hmac::{Hmac, Mac};
use ops_core::db::Database;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudInstance {
    pub instance_id: String,
    pub name: String,
    pub ip: String,
    pub inner_ip: String,
    pub os: String,
    pub status: String,
    pub region: String,
    pub instance_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudProvider {
    pub provider: String,
    pub name: String,
    pub access_key_id: String,
    pub access_key_secret: String,
    pub regions: Vec<String>,
}
pub fn list_cloud_providers(db: &Database) -> Result<Vec<CloudProvider>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let result = conn.query_row(
        "SELECT value FROM settings WHERE key='cloud_providers'",
        [],
        |row| row.get::<_, String>(0),
    );
    match result {
        Ok(json) => serde_json::from_str(&json).map_err(|e| e.to_string()),
        Err(_) => Ok(Vec::new()),
    }
}
pub fn save_cloud_providers(db: &Database, providers: Vec<CloudProvider>) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&providers).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('cloud_providers', ?1)",
        params![json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn fetch_cloud_instances(
    db: &Database,
    provider: String,
    region: String,
) -> Result<Vec<CloudInstance>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let json = conn
        .query_row(
            "SELECT value FROM settings WHERE key='cloud_providers'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|_| "请先在设置中配置云平台凭据".to_string())?;
    let providers: Vec<CloudProvider> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    drop(conn);

    let cred = providers
        .iter()
        .find(|p| p.provider == provider)
        .ok_or_else(|| format!("未找到 {} 的凭据配置", provider))?;

    match provider.as_str() {
        "aliyun" => fetch_aliyun(cred, &region),
        "aws" => fetch_aws(cred, &region),
        "tencent" => fetch_tencent(cred, &region),
        _ => Err(format!("不支持的云平台: {}", provider)),
    }
}

// ============================================================
// Alibaba Cloud ECS — HMAC-SHA1 signature
// ============================================================

fn percent_encode(s: &str) -> String {
    // RFC 3986: encode every UTF-8 *byte* (the Tauri build cast whole chars to u8, mangling non-ASCII input).
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
                (b as char).to_string()
            } else {
                format!("%{:02X}", b)
            }
        })
        .collect()
}

fn hmac_sha1(key: &[u8], data: &[u8]) -> Vec<u8> {
    // HMAC-SHA1 implementation for Alibaba Cloud signatures
    // so we use a simple HMAC-SHA1 implementation
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    let key_padded = {
        let mut k = [0u8; 64];
        if key.len() > 64 {
            let digest = sha1_hash(key);
            k[..20].copy_from_slice(&digest);
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        k
    };
    for i in 0..64 {
        ipad[i] ^= key_padded[i];
        opad[i] ^= key_padded[i];
    }
    let mut inner = ipad.to_vec();
    inner.extend_from_slice(data);
    let inner_hash = sha1_hash(&inner);
    let mut outer = opad.to_vec();
    outer.extend_from_slice(&inner_hash);
    sha1_hash(&outer).to_vec()
}

// SHA-1 标准算法：按轮次索引 w[] 是算法固有写法，反迭代器化反而降低可读性。
#[allow(clippy::needless_range_loop)]
fn sha1_hash(data: &[u8]) -> [u8; 20] {
    // Simple SHA-1 implementation
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let ml = data.len() as u64 * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&ml.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut result = [0u8; 20];
    for (i, v) in [h0, h1, h2, h3, h4].iter().enumerate() {
        result[i * 4..(i + 1) * 4].copy_from_slice(&v.to_be_bytes());
    }
    result
}

fn fetch_aliyun(cred: &CloudProvider, region: &str) -> Result<Vec<CloudInstance>, String> {
    let endpoint = format!("ecs.{}.aliyuncs.com", region);
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let nonce = uuid::Uuid::new_v4().to_string();

    let mut params: BTreeMap<String, String> = BTreeMap::new();
    params.insert("Action".into(), "DescribeInstances".into());
    params.insert("Version".into(), "2014-05-26".into());
    params.insert("AccessKeyId".into(), cred.access_key_id.clone());
    params.insert("SignatureMethod".into(), "HMAC-SHA1".into());
    params.insert("Timestamp".into(), timestamp);
    params.insert("SignatureVersion".into(), "1.0".into());
    params.insert("SignatureNonce".into(), nonce);
    params.insert("Format".into(), "JSON".into());
    params.insert("RegionId".into(), region.into());
    params.insert("PageSize".into(), "100".into());

    let canonical_query: String = params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    let string_to_sign = format!("GET&%2F&{}", percent_encode(&canonical_query));
    let signing_key = format!("{}&", cred.access_key_secret);
    let signature = base64::engine::general_purpose::STANDARD
        .encode(hmac_sha1(signing_key.as_bytes(), string_to_sign.as_bytes()));

    let url = format!(
        "https://{}?{}&Signature={}",
        endpoint,
        canonical_query,
        percent_encode(&signature)
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("请求阿里云API失败: {}", e))?;
    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("解析响应失败 ({}): {}", status, e))?;

    if status.as_u16() >= 400 {
        let err_msg = body["Message"].as_str().unwrap_or("未知错误");
        return Err(format!("阿里云API错误: {}", err_msg));
    }

    let mut instances = Vec::new();
    if let Some(insts) = body["Instances"]["Instance"].as_array() {
        for inst in insts {
            let public_ip = inst["PublicIpAddress"]["IpAddress"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let inner_ip = inst["VpcAttributes"]["PrivateIpAddress"]["IpAddress"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .unwrap_or("");
            instances.push(CloudInstance {
                instance_id: inst["InstanceId"].as_str().unwrap_or("").into(),
                name: inst["InstanceName"].as_str().unwrap_or("").into(),
                ip: public_ip.into(),
                inner_ip: inner_ip.into(),
                os: inst["OSType"].as_str().unwrap_or("linux").into(),
                status: inst["Status"].as_str().unwrap_or("unknown").into(),
                region: region.into(),
                instance_type: inst["InstanceType"].as_str().unwrap_or("").into(),
            });
        }
    }
    Ok(instances)
}

// ============================================================
// AWS EC2 — AWS SigV4 signature
// ============================================================

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_raw(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

fn hex_sha256(data: &[u8]) -> String {
    sha256_raw(data)
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

// Simplified AWS implementation using query-style API with manual signing
fn fetch_aws(cred: &CloudProvider, region: &str) -> Result<Vec<CloudInstance>, String> {
    let service = "ec2";
    let host = format!("ec2.{}.amazonaws.com", region);
    let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = chrono::Utc::now().format("%Y%m%d").to_string();

    // Canonical request
    let method = "GET";
    let canonical_uri = "/";
    let canonical_querystring = "Action=DescribeInstances&Version=2016-11-15";
    let canonical_headers = format!("host:{}\nx-amz-date:{}\n", host, amz_date);
    let signed_headers = "host;x-amz-date";
    let payload_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"; // SHA256 of empty string

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method,
        canonical_uri,
        canonical_querystring,
        canonical_headers,
        signed_headers,
        payload_hash
    );

    // String to sign
    let algorithm = "AWS4-HMAC-SHA256";
    let credential_scope = format!("{}/{}/{}/aws4_request", date_stamp, region, service);
    let string_to_sign = format!(
        "{}\n{}\n{}\n{}",
        algorithm,
        amz_date,
        credential_scope,
        hex_sha256(canonical_request.as_bytes())
    );

    // Signing key
    let k_date = hmac_sha256(
        format!("AWS4{}", cred.access_key_secret).as_bytes(),
        date_stamp.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hmac_sha256(&k_signing, string_to_sign.as_bytes());

    let authorization = format!(
        "{} Credential={}/{}, SignedHeaders={}, Signature={}",
        algorithm,
        cred.access_key_id,
        credential_scope,
        signed_headers,
        signature
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    );

    let url = format!("https://{}?{}", host, canonical_querystring);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .header("Host", &host)
        .header("X-Amz-Date", &amz_date)
        .header("Authorization", &authorization)
        .send()
        .map_err(|e| format!("请求AWS API失败: {}", e))?;

    let status = resp.status();
    let body_text = resp.text().map_err(|e| format!("读取响应失败: {}", e))?;

    // AWS returns XML, parse basic instance info
    let mut instances = Vec::new();
    // Simple XML parsing for instanceId, privateIpAddress, publicIpAddress, instanceState
    let instance_blocks: Vec<&str> = body_text.split("<item>").skip(1).collect();
    for block in &instance_blocks {
        let get_tag = |tag: &str| -> String {
            let start_tag = format!("<{}>", tag);
            let end_tag = format!("</{}>", tag);
            if let Some(s) = block.find(&start_tag) {
                let content = &block[s + start_tag.len()..];
                if let Some(e) = content.find(&end_tag) {
                    return content[..e].to_string();
                }
            }
            String::new()
        };
        let instance_id = get_tag("instanceId");
        if instance_id.is_empty() {
            continue;
        }
        let private_ip = get_tag("privateIpAddress");
        let public_ip = get_tag("ipAddress");
        if public_ip.is_empty() {
            // Try nested in associations
            let _ = block.find("<item>").map(|_| {});
        }
        let state = get_tag("name");
        let inst_type = get_tag("instanceType");

        instances.push(CloudInstance {
            instance_id,
            name: {
                let display = get_tag("displayName");
                if display.is_empty() {
                    get_tag("value")
                } else {
                    display
                }
            },
            ip: if public_ip.is_empty() {
                private_ip.clone()
            } else {
                public_ip
            },
            inner_ip: private_ip,
            os: "linux".into(),
            status: state,
            region: region.into(),
            instance_type: inst_type,
        });
    }

    if status.as_u16() >= 400 {
        return Err(format!(
            "AWS API错误: {}",
            &body_text[..body_text.len().min(200)]
        ));
    }
    Ok(instances)
}

// ============================================================
// Tencent Cloud CVM — TC3-HMAC-SHA256 signature
// ============================================================

fn fetch_tencent(cred: &CloudProvider, region: &str) -> Result<Vec<CloudInstance>, String> {
    let service = "cvm";
    let host = format!("cvm.{}.tencentcloudapi.com", region);
    let action = "DescribeInstances";
    let version = "2017-03-12";
    let timestamp = chrono::Utc::now().timestamp().to_string();

    let payload = r#"{"Limit":100,"Filters":[]}"#;

    // Canonical request
    let http_method = "POST";
    let canonical_uri = "/";
    let canonical_qs = "";
    let ct = "application/json; charset=utf-8";
    let canonical_headers = format!("content-type:{}\nhost:{}\n", ct, host);
    let signed_headers = "content-type;host";
    let hashed_payload = hex_sha256(payload.as_bytes());

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        http_method, canonical_uri, canonical_qs, canonical_headers, signed_headers, hashed_payload
    );

    // String to sign
    let algorithm = "TC3-HMAC-SHA256";
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let credential_scope = format!("{}/{}/{}/tc3_request", date, service, service);
    let string_to_sign = format!(
        "{}\n{}\n{}\n{}",
        algorithm,
        timestamp,
        credential_scope,
        hex_sha256(canonical_request.as_bytes())
    );

    // Signing key
    let secret_date = hmac_sha256(
        format!("TC3{}", cred.access_key_secret).as_bytes(),
        date.as_bytes(),
    );
    let secret_service = hmac_sha256(&secret_date, service.as_bytes());
    let secret_signing = hmac_sha256(&secret_service, b"tc3_request");
    let signature = hmac_sha256(&secret_signing, string_to_sign.as_bytes());

    let authorization = format!(
        "{} Credential={}/{}, SignedHeaders={}, Signature={}",
        algorithm,
        cred.access_key_id,
        credential_scope,
        signed_headers,
        signature
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    );

    let url = format!("https://{}", host);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(&url)
        .header("Content-Type", ct)
        .header("Host", &host)
        .header("X-TC-Action", action)
        .header("X-TC-Version", version)
        .header("X-TC-Timestamp", &timestamp)
        .header("X-TC-Region", region)
        .header("Authorization", &authorization)
        .body(payload.to_string())
        .send()
        .map_err(|e| format!("请求腾讯云API失败: {}", e))?;

    let status = resp.status();
    let body: serde_json::Value = resp.json().map_err(|e| format!("解析响应失败: {}", e))?;

    if status.as_u16() >= 400 || body["Response"]["Error"].is_object() {
        let err = body["Response"]["Error"]["Message"]
            .as_str()
            .unwrap_or("未知错误");
        return Err(format!("腾讯云API错误: {}", err));
    }

    let mut instances = Vec::new();
    if let Some(insts) = body["Response"]["InstanceSet"].as_array() {
        for inst in insts {
            let public_ip = inst["PublicIpAddresses"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let private_ip = inst["PrivateIpAddresses"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            instances.push(CloudInstance {
                instance_id: inst["InstanceId"].as_str().unwrap_or("").into(),
                name: inst["InstanceName"].as_str().unwrap_or("").into(),
                ip: if public_ip.is_empty() {
                    private_ip.clone()
                } else {
                    public_ip
                },
                inner_ip: private_ip,
                os: "linux".into(),
                status: inst["InstanceState"].as_str().unwrap_or("unknown").into(),
                region: region.into(),
                instance_type: inst["InstanceType"].as_str().unwrap_or("").into(),
            });
        }
    }
    Ok(instances)
}
pub fn import_cloud_instances(
    db: &Database,
    instances: Vec<CloudInstance>,
    group_id: Option<String>,
) -> Result<u32, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut imported = 0u32;

    for inst in &instances {
        let id = uuid::Uuid::new_v4().to_string();
        let ip = if inst.ip.is_empty() {
            &inst.inner_ip
        } else {
            &inst.ip
        };
        if ip.is_empty() {
            continue;
        }

        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM hosts WHERE ip=?1",
                params![ip],
                |row| row.get::<_, i32>(0).map(|c| c > 0),
            )
            .unwrap_or(false);
        if exists {
            continue;
        }

        let os = if inst.os.to_lowercase().contains("windows") {
            "windows"
        } else {
            "linux"
        };
        let name = if inst.name.is_empty() {
            format!("{}-{}", inst.region, inst.instance_id)
        } else {
            inst.name.clone()
        };

        conn.execute(
            "INSERT INTO hosts (id, name, ip, port, auth_type, username, password, private_key, os, tags, group_id, remark) VALUES (?1, ?2, ?3, 22, 'password', 'root', NULL, NULL, ?4, ?5, ?6, ?7)",
            params![
                id, name, ip, os,
                format!("[\"{}\"]", inst.region),
                group_id,
                format!("{} {} {}", inst.region, inst.instance_type, inst.instance_id),
            ],
        ).map_err(|e| e.to_string())?;
        imported += 1;
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha1_matches_fips_180_vectors() {
        assert_eq!(
            hex(&sha1_hash(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1_hash(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            hex(&sha1_hash(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    #[test]
    fn hmac_sha1_matches_rfc2202_vectors() {
        assert_eq!(
            hex(&hmac_sha1(&[0x0b; 20], b"Hi There")),
            "b617318655057264e28bc0b6fb378c8ef146be00"
        );
        assert_eq!(
            hex(&hmac_sha1(b"Jefe", b"what do ya want for nothing?")),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
        // key longer than the block size is hashed first
        assert_eq!(
            hex(&hmac_sha1(
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "aa4ae5e15272d00e95705637ce8a3b55ed402112"
        );
    }

    #[test]
    fn percent_encode_follows_rfc3986_for_aliyun_signatures() {
        assert_eq!(
            percent_encode("a b&c=d/e~f-g_h.i*j"),
            "a%20b%26c%3Dd%2Fe~f-g_h.i%2Aj"
        );
        assert_eq!(percent_encode("中"), "%E4%B8%AD");
    }

    #[test]
    fn providers_are_stored_in_settings_and_listed_back() {
        let core = ops_core::Core::open_temp("cloud-providers").unwrap();
        assert!(list_cloud_providers(&core.db).unwrap().is_empty());
        let p = CloudProvider {
            provider: "aliyun".into(),
            name: "prod".into(),
            access_key_id: "AK".into(),
            access_key_secret: "SK".into(),
            regions: vec!["cn-hangzhou".into()],
        };
        save_cloud_providers(&core.db, vec![p.clone()]).unwrap();
        let back = list_cloud_providers(&core.db).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(
            (back[0].provider.as_str(), back[0].regions.clone()),
            ("aliyun", vec!["cn-hangzhou".to_string()])
        );
        // fetching without credentials for that provider fails with a clear message, without any network call
        let err = fetch_cloud_instances(&core.db, "aws".into(), "us-east-1".into()).unwrap_err();
        assert!(err.contains("aws"), "{err}");
        let err = fetch_cloud_instances(&core.db, "gcp".into(), "x".into()).unwrap_err();
        assert!(err.contains("gcp"), "{err}");
    }

    fn inst(id: &str, name: &str, ip: &str, inner: &str, os: &str) -> CloudInstance {
        CloudInstance {
            instance_id: id.into(),
            name: name.into(),
            ip: ip.into(),
            inner_ip: inner.into(),
            os: os.into(),
            status: "Running".into(),
            region: "cn-hangzhou".into(),
            instance_type: "ecs.c6.large".into(),
        }
    }

    #[test]
    fn import_creates_hosts_skips_duplicates_and_empty_addresses() {
        let core = ops_core::Core::open_temp("cloud-import").unwrap();
        let group = {
            let conn = core.db.pool.get().unwrap();
            conn.execute(
                "INSERT INTO host_groups (id, name) VALUES ('g1', 'cloud')",
                [],
            )
            .unwrap();
            "g1".to_string()
        };
        let batch = vec![
            inst("i-1", "web", "1.1.1.1", "10.0.0.1", "Ubuntu 22.04"),
            inst("i-2", "", "", "10.0.0.2", "Windows Server 2019"),
            inst("i-3", "ghost", "", "", "linux"),
            inst("i-4", "dup", "1.1.1.1", "", "linux"),
        ];
        let n = import_cloud_instances(&core.db, batch.clone(), Some(group.clone())).unwrap();
        assert_eq!(n, 2, "i-3 has no address, i-4 duplicates 1.1.1.1");
        let conn = core.db.pool.get().unwrap();
        let rows: Vec<(String, String, String, Option<String>, String)> = conn
            .prepare("SELECT name, ip, os, group_id, remark FROM hosts ORDER BY ip")
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows[0].0, "web");
        assert_eq!(
            (rows[0].1.as_str(), rows[0].2.as_str(), rows[0].3.as_deref()),
            ("1.1.1.1", "linux", Some("g1"))
        );
        assert_eq!(
            rows[1].0, "cn-hangzhou-i-2",
            "unnamed instances use region-id"
        );
        assert_eq!(
            (rows[1].1.as_str(), rows[1].2.as_str()),
            ("10.0.0.2", "windows"),
            "falls back to the inner ip"
        );
        assert!(rows[1].4.contains("ecs.c6.large") && rows[1].4.contains("i-2"));
        drop(conn);
        assert_eq!(
            import_cloud_instances(&core.db, batch, None).unwrap(),
            0,
            "second import adds nothing"
        );
    }
}
