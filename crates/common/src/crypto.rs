use anyhow::{anyhow, Result};

/// AES-256-GCM 加密器，用于保护存储在数据库中的敏感配置（如 SSH 密码、SOAP 凭证等）。
/// 无 ENCRYPTION_KEY 环境变量时 from_env() 返回 None，系统自动降级为明文存储，
/// 确保开发环境零配置即可运行。
pub struct Encryptor {
    key: [u8; 32],
}

impl Encryptor {
    /// 从环境变量 ENCRYPTION_KEY 读取 64 位 hex 编码的 256-bit 密钥。
    /// 返回 None 表示未配置加密，调用方应降级为明文存储。
    pub fn from_env() -> Option<Self> {
        let key_hex = std::env::var("ENCRYPTION_KEY").ok()?;
        let key_bytes = hex_decode(&key_hex).ok()?;
        if key_bytes.len() != 32 {
            return None;
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&key_bytes);
        Some(Self { key })
    }

    /// 加密明文为 hex 编码的密文。
    /// 输出格式：nonce(24 hex chars / 12 bytes) + ciphertext + tag(32 hex chars / 16 bytes)。
    /// 每次加密使用随机 nonce，相同明文产生不同密文。
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
        use ring::rand::{SecureRandom, SystemRandom};

        let rng = SystemRandom::new();
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rng.fill(&mut nonce_bytes)
            .map_err(|_| anyhow!("RNG failed"))?;

        let unbound =
            UnboundKey::new(&AES_256_GCM, &self.key).map_err(|_| anyhow!("Invalid key"))?;
        let key = LessSafeKey::new(unbound);
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let mut in_out = plaintext.as_bytes().to_vec();
        key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| anyhow!("Encryption failed"))?;

        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&in_out);
        Ok(hex_encode(&result))
    }

    /// 解密 hex 编码的密文为明文。
    /// 密文格式必须与 encrypt() 输出一致：nonce + ciphertext + tag。
    pub fn decrypt(&self, ciphertext_hex: &str) -> Result<String> {
        use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};

        let data = hex_decode(ciphertext_hex)?;
        if data.len() < NONCE_LEN + 16 {
            return Err(anyhow!("Ciphertext too short"));
        }

        let (nonce_bytes, encrypted) = data.split_at(NONCE_LEN);
        let nonce = Nonce::assume_unique_for_key(nonce_bytes.try_into().unwrap());

        let unbound =
            UnboundKey::new(&AES_256_GCM, &self.key).map_err(|_| anyhow!("Invalid key"))?;
        let key = LessSafeKey::new(unbound);

        let mut in_out = encrypted.to_vec();
        let plaintext = key
            .open_in_place(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| anyhow!("Decryption failed"))?;

        Ok(String::from_utf8(plaintext.to_vec())?)
    }
}

fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_decode(hex: &str) -> Result<Vec<u8>> {
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|e| anyhow!("Invalid hex: {}", e))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_encryptor() -> Encryptor {
        let key_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let key_bytes = hex_decode(key_hex).unwrap();
        let mut key = [0u8; 32];
        key.copy_from_slice(&key_bytes);
        Encryptor { key }
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let enc = test_encryptor();
        let plaintext = "ssh-password-123!@#";
        let ciphertext = enc.encrypt(plaintext).unwrap();
        assert_ne!(ciphertext, plaintext);
        let decrypted = enc.decrypt(&ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_produces_different_ciphertext_each_time() {
        let enc = test_encryptor();
        let c1 = enc.encrypt("test").unwrap();
        let c2 = enc.encrypt("test").unwrap();
        // 随机 nonce 保证相同明文每次产生不同密文
        assert_ne!(c1, c2);
    }

    #[test]
    fn decrypt_invalid_hex_fails() {
        let enc = test_encryptor();
        assert!(enc.decrypt("not-hex!").is_err());
    }

    #[test]
    fn decrypt_too_short_fails() {
        let enc = test_encryptor();
        assert!(enc.decrypt("aabbccdd").is_err());
    }

    #[test]
    fn from_env_returns_none_without_key() {
        assert!(Encryptor::from_env().is_none());
    }
}
