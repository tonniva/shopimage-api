use anyhow::{anyhow, Result};
use azure_storage_blobs::prelude::*;
use azure_storage::prelude::*;
use azure_core::prelude::*;

pub struct BlobUploader {
    container: String,
    service: BlobServiceClient,
}

impl BlobUploader {
    pub fn from_connection_string(conn_str: &str, container: &str) -> Result<Self> {
        // สร้าง service client จาก connection string
        let service = BlobServiceClient::from_connection_string(conn_str)
            .map_err(|e| anyhow!("invalid connection string: {e}"))?;
        Ok(Self {
            container: container.to_string(),
            service,
        })
    }

    pub async fn upload_bytes(
        &self,
        blob_path: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<()> {
        let container = self.service.container_client(&self.container);
        let blob = container.blob_client(blob_path);

        // สร้าง container หากยังไม่มี (ignore 409)
        let _ = container.create().public_access(PublicAccess::None).into_future().await;

        // อัปโหลดแบบ block blob
        blob.put_block_blob(bytes.to_vec())
            .content_type(content_type)
            .into_future()
            .await
            .map_err(|e| anyhow!("upload error: {e}"))?;
        Ok(())
    }

    /// (ทางเลือกในอนาคต) สร้าง SAS URL แบบอ่านได้ชั่วคราว
    /// ตอนนี้เรายังไม่เรียกใช้ เพื่อให้แน่ใจว่าคอมไพล์ผ่านทุกสภาพแวดล้อม
    #[allow(dead_code)]
    pub fn make_blob_url(&self, blob_path: &str) -> String {
        let account = self.service.storage_account_client().account();
        format!(
            "https://{account}.blob.core.windows.net/{}/{}",
            self.container,
            blob_path
        )
    }
}