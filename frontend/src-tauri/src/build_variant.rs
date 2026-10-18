//! Standard build vs classified (涉密版) build.
//!
//! The standard build enables the `online` Cargo feature (on by default). The classified build is
//! compiled with `--no-default-features`, which leaves out at compile time:
//! - the cloud summary providers (OpenAI, Claude, Groq, OpenRouter) and their model lists,
//! - PostHog analytics (`posthog-rs` is not linked),
//! - the auto-updater (`tauri-plugin-updater` is not linked).
//!
//! Ollama and custom OpenAI-compatible servers stay available in the classified build, but only
//! on this machine or the internal network: see [`check_endpoint`].

/// Shown when a cloud provider is requested in the classified build
#[cfg_attr(feature = "online", allow(dead_code))]
pub const CLOUD_DISABLED: &str = "涉密版不包含云端模型，请使用内置模型、本机 Ollama 或内网接口";

/// Whether this is the classified build
#[tauri::command]
pub fn is_classified_build() -> bool {
    !cfg!(feature = "online")
}

/// In the classified build, a model server must be on this machine or the internal network:
/// loopback or private IP addresses, single-label host names (`llm-server`), or names under
/// `.local`, `.lan`, `.internal`, `.intranet`. Public addresses and other domain names are refused.
/// The standard build accepts any address.
pub fn check_endpoint(endpoint: &str) -> Result<(), String> {
    if cfg!(feature = "online") || endpoint.trim().is_empty() {
        return Ok(());
    }
    if is_internal_endpoint(endpoint) {
        Ok(())
    } else {
        Err(format!(
            "涉密版只能连接本机或内网的模型服务，“{}”不是内网地址。请填写内网 IP（如 http://192.168.1.10:11434）或内网主机名",
            endpoint.trim()
        ))
    }
}

pub fn is_internal_endpoint(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint.trim()) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Some(url::Host::Ipv6(ip)) => {
            // ::1, and unique local fc00::/7
            ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00
        }
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            name == "localhost"
                || !name.contains('.')
                || [".localhost", ".local", ".lan", ".internal", ".intranet"].iter().any(|suffix| name.ends_with(suffix))
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_endpoints_are_recognized() {
        for ok in [
            "http://localhost:11434",
            "http://127.0.0.1:11434",
            "http://[::1]:8000/v1",
            "http://192.168.1.10:11434",
            "http://10.0.0.5/v1",
            "http://172.20.1.1:8000",
            "http://llm-server:8000/v1",
            "http://gpu01.intranet/v1",
            "http://[fd00::1]:8000",
        ] {
            assert!(is_internal_endpoint(ok), "{ok} should be internal");
        }
        for bad in [
            "https://api.deepseek.com/v1",
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "http://8.8.8.8:11434",
            "http://172.32.0.1",
            "not a url",
        ] {
            assert!(!is_internal_endpoint(bad), "{bad} should not be internal");
        }
    }
}
