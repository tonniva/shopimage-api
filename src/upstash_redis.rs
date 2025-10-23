use base64::{engine::general_purpose, Engine as _};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;

pub struct UpstashRedis {
    client: Client,
    base_url: String,
    token: String,
}

#[derive(Debug)]
pub struct CacheResult {
    pub data: Vec<u8>,
    pub content_type: String,
    pub filename: String,
    pub size_kb: u64,
}

impl UpstashRedis {
    pub fn new() -> Result<Self, String> {
        let base_url = std::env::var("UPSTASH_REDIS_REST_URL")
            .map_err(|_| "Missing UPSTASH_REDIS_REST_URL".to_string())?;
        let token = std::env::var("UPSTASH_REDIS_REST_TOKEN")
            .map_err(|_| "Missing UPSTASH_REDIS_REST_TOKEN".to_string())?;

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        Ok(Self {
            client,
            base_url,
            token,
        })
    }

    pub async fn test_connection(&self) -> Result<(), String> {
        let test_key = "test_connection";
        let test_value = "test_value";

        // Test SET
        self.set(test_key, test_value, 60).await?;

        // Test GET
        let result = self.get(test_key).await?;
        if result != test_value {
            return Err(format!(
                "Redis test failed: expected '{}', got '{}'",
                test_value, result
            ));
        }

        // Clean up
        let _ = self.del(test_key).await;

        println!("✅ Redis connection test successful");
        Ok(())
    }

    async fn set(&self, key: &str, value: &str, ttl_seconds: u64) -> Result<(), String> {
        let url = format!("{}/set/{}", self.base_url, urlencoding::encode(key));
        let body = json!({
            "value": value,
            "ex": ttl_seconds
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Redis SET failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Redis SET failed: {} - {}", status, error_text));
        }

        Ok(())
    }

    async fn get(&self, key: &str) -> Result<String, String> {
        let url = format!("{}/get/{}", self.base_url, urlencoding::encode(key));

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .map_err(|e| format!("Redis GET failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Redis GET failed: {} - {}", status, error_text));
        }

        let text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        // Handle Upstash REST API response format
        if text == "null" {
            return Err("Key not found".to_string());
        }

        // Try to parse as JSON first (Upstash format)
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            // Handle nested JSON format: {"result":"{\"ex\":60,\"value\":\"test_value\"}"}
            if let Some(result_str) = json.get("result").and_then(|v| v.as_str()) {
                if let Ok(nested_json) = serde_json::from_str::<serde_json::Value>(result_str) {
                    if let Some(value) = nested_json.get("value").and_then(|v| v.as_str()) {
                        return Ok(value.to_string());
                    }
                }
            }

            // Handle direct format: {"value":"test_value"}
            if let Some(value) = json.get("value").and_then(|v| v.as_str()) {
                return Ok(value.to_string());
            }
        }

        // Fallback to direct string
        Ok(text)
    }

    async fn del(&self, key: &str) -> Result<(), String> {
        let url = format!("{}/del/{}", self.base_url, urlencoding::encode(key));

        let response = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .map_err(|e| format!("Redis DEL failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Redis DEL failed: {} - {}", status, error_text));
        }

        Ok(())
    }

    pub async fn cache_image_result(
        &self,
        file_hash: &str,
        target_w: Option<u32>,
        target_h: Option<u32>,
        format: &str,
        result: &CacheResult,
    ) -> Result<(), String> {
        let cache_key = format!(
            "convert:{}:{}:{}:{}",
            file_hash,
            target_w.unwrap_or(0),
            target_h.unwrap_or(0),
            format
        );

        let cache_data = json!({
            "data": general_purpose::STANDARD.encode(&result.data),
            "content_type": result.content_type,
            "filename": result.filename,
            "size_kb": result.size_kb
        });

        self.set(&cache_key, &cache_data.to_string(), 3600).await?; // 1 hour TTL
        println!("💾 Cached result with key: {}", cache_key);
        Ok(())
    }

    pub async fn get_cached_image_result(
        &self,
        file_hash: &str,
        target_w: Option<u32>,
        target_h: Option<u32>,
        format: &str,
    ) -> Result<Option<CacheResult>, String> {
        let cache_key = format!(
            "convert:{}:{}:{}:{}",
            file_hash,
            target_w.unwrap_or(0),
            target_h.unwrap_or(0),
            format
        );

        match self.get(&cache_key).await {
            Ok(cached_json) => {
                let cache_data: serde_json::Value = serde_json::from_str(&cached_json)
                    .map_err(|e| format!("Failed to parse cached data: {}", e))?;

                let data = general_purpose::STANDARD
                    .decode(
                        cache_data
                            .get("data")
                            .and_then(|v| v.as_str())
                            .ok_or("Missing data field")?,
                    )
                    .map_err(|e| format!("Failed to decode base64: {}", e))?;

                let result = CacheResult {
                    data,
                    content_type: cache_data
                        .get("content_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("image/webp")
                        .to_string(),
                    filename: cache_data
                        .get("filename")
                        .and_then(|v| v.as_str())
                        .unwrap_or("cached_image.webp")
                        .to_string(),
                    size_kb: cache_data
                        .get("size_kb")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                };

                println!("🎯 Cache hit for key: {}", cache_key);
                Ok(Some(result))
            }
            Err(_) => {
                println!("💭 Cache miss for key: {}", cache_key);
                Ok(None)
            }
        }
    }

    pub async fn cache_remove_bg_result(
        &self,
        file_hash: &str,
        border_size: u64,
        border_color: &str,
        result: &CacheResult,
    ) -> Result<(), String> {
        let cache_key = format!("remove_bg:{}:{}:{}", file_hash, border_size, border_color);

        let cache_data = json!({
            "data": general_purpose::STANDARD.encode(&result.data),
            "content_type": result.content_type,
            "filename": result.filename,
            "size_kb": result.size_kb
        });

        self.set(&cache_key, &cache_data.to_string(), 7200).await?; // 2 hours TTL
        println!("💾 Cached remove-bg result with key: {}", cache_key);
        Ok(())
    }

    pub async fn get_cached_remove_bg_result(
        &self,
        file_hash: &str,
        border_size: u64,
        border_color: &str,
    ) -> Result<Option<CacheResult>, String> {
        let cache_key = format!("remove_bg:{}:{}:{}", file_hash, border_size, border_color);

        match self.get(&cache_key).await {
            Ok(cached_json) => {
                let cache_data: serde_json::Value = serde_json::from_str(&cached_json)
                    .map_err(|e| format!("Failed to parse cached remove-bg data: {}", e))?;

                let data = general_purpose::STANDARD
                    .decode(
                        cache_data
                            .get("data")
                            .and_then(|v| v.as_str())
                            .ok_or("Missing data field")?,
                    )
                    .map_err(|e| format!("Failed to decode base64: {}", e))?;

                let result = CacheResult {
                    data,
                    content_type: cache_data
                        .get("content_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("image/png")
                        .to_string(),
                    filename: cache_data
                        .get("filename")
                        .and_then(|v| v.as_str())
                        .unwrap_or("cached_nobg.png")
                        .to_string(),
                    size_kb: cache_data
                        .get("size_kb")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                };

                println!("🎯 Remove-bg cache hit for key: {}", cache_key);
                Ok(Some(result))
            }
            Err(_) => {
                println!("💭 Remove-bg cache miss for key: {}", cache_key);
                Ok(None)
            }
        }
    }

    pub async fn cache_pdf_all_result(
        &self,
        pdf_hash: &str,
        result: &CacheResult,
    ) -> Result<(), String> {
        let cache_key = format!("pdf_all:{}", pdf_hash);

        let cache_data = json!({
            "data": general_purpose::STANDARD.encode(&result.data),
            "content_type": result.content_type,
            "filename": result.filename,
            "size_kb": result.size_kb
        });

        self.set(&cache_key, &cache_data.to_string(), 7200).await?; // 2 hours TTL
        println!("💾 Cached PDF-all result with key: {}", cache_key);
        Ok(())
    }

    pub async fn get_cached_pdf_all_result(
        &self,
        pdf_hash: &str,
    ) -> Result<Option<CacheResult>, String> {
        let cache_key = format!("pdf_all:{}", pdf_hash);

        match self.get(&cache_key).await {
            Ok(cached_json) => {
                let cache_data: serde_json::Value = serde_json::from_str(&cached_json)
                    .map_err(|e| format!("Failed to parse cached PDF-all data: {}", e))?;

                let data = general_purpose::STANDARD
                    .decode(
                        cache_data
                            .get("data")
                            .and_then(|v| v.as_str())
                            .ok_or("Missing data field")?,
                    )
                    .map_err(|e| format!("Failed to decode base64: {}", e))?;

                let result = CacheResult {
                    data,
                    content_type: cache_data
                        .get("content_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("application/pdf")
                        .to_string(),
                    filename: cache_data
                        .get("filename")
                        .and_then(|v| v.as_str())
                        .unwrap_or("cached_pdf_all.pdf")
                        .to_string(),
                    size_kb: cache_data
                        .get("size_kb")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                };

                println!("🎯 PDF-all cache hit for key: {}", cache_key);
                Ok(Some(result))
            }
            Err(_) => {
                println!("💭 PDF-all cache miss for key: {}", cache_key);
                Ok(None)
            }
        }
    }

    // Cache functions for convert-pdf endpoint
    pub async fn cache_pdf_result(
        &self,
        pdf_hash: &str,
        page: u32,
        result: &CacheResult,
    ) -> Result<(), String> {
        let cache_key = format!("pdf:{}:{}", pdf_hash, page);
        let ttl_seconds = 7200; // 2 hours

        let cache_data = serde_json::json!({
            "data": base64::engine::general_purpose::STANDARD.encode(&result.data),
            "content_type": result.content_type,
            "filename": result.filename,
            "size_kb": result.size_kb,
        });

        self.set(&cache_key, &cache_data.to_string(), ttl_seconds)
            .await
            .map_err(|e| format!("Failed to cache PDF result: {}", e))?;

        println!("✅ PDF result cached with key: {}", cache_key);
        Ok(())
    }

    pub async fn get_cached_pdf_result(
        &self,
        pdf_hash: &str,
        page: u32,
    ) -> Result<Option<CacheResult>, String> {
        let cache_key = format!("pdf:{}:{}", pdf_hash, page);

        match self.get(&cache_key).await {
            Ok(cached_data) => {
                let cache_data: serde_json::Value = serde_json::from_str(&cached_data)
                    .map_err(|e| format!("Failed to parse cached PDF data: {}", e))?;

                let data = cache_data
                    .get("data")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing data field")?;

                let data = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|e| format!("Failed to decode cached PDF data: {}", e))?;

                let result = CacheResult {
                    data,
                    content_type: cache_data
                        .get("content_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("image/jpeg")
                        .to_string(),
                    filename: cache_data
                        .get("filename")
                        .and_then(|v| v.as_str())
                        .unwrap_or("cached_pdf.jpg")
                        .to_string(),
                    size_kb: cache_data
                        .get("size_kb")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                };

                println!("🎯 PDF cache hit for key: {}", cache_key);
                Ok(Some(result))
            }
            Err(_) => {
                println!("💭 PDF cache miss for key: {}", cache_key);
                Ok(None)
            }
        }
    }
}

pub fn calculate_file_hash(data: &[u8]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}
