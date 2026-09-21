//! 运行期路径推导：应用私有 `known_hosts` 与隧道诊断日志。
//!
//! # 为什么放在 core
//!
//! 这些路径原先由**每个壳层各自推导**，而 [`StartOptions`] 的语义是「壳层负责提供
//! 平台相关路径与凭据」。只要壳层漏填，OpenSSH 就会退回默认行为：
//! `known_hosts` 落回用户主目录的 `~/.ssh/known_hosts`（污染系统级信任记录），
//! 且不产生任何诊断日志。
//!
//! 这个缺陷真实发生过：桌面壳层（`apps/desktop`）在 0.1.17 修好了，
//! 命令行壳层（`apps/cli`）却一直传 [`StartOptions::default`]，于是 CLI 启动的隧道
//! 仍在写 `~/.ssh/known_hosts`、且没有任何日志。
//!
//! 把推导集中到 core 后，两个壳层共用同一实现，**结构上无法再各自遗漏**；
//! [`RuntimePaths::options`] 是组装 [`StartOptions`] 的推荐入口。

use std::path::{Path, PathBuf};

use crate::StartOptions;

/// 应用私有数据目录（与配置文件同级）。
///
/// 配置文件没有父目录时（例如只给了 `config.json`）退化为当前目录，
/// 这样后续 join 出来的仍是相对路径而不是绝对路径。
pub fn app_data_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 应用私有 `known_hosts`。
///
/// 刻意**不使用** `~/.ssh/known_hosts`：应用不应替用户决定系统级的信任记录。
/// 与配置文件同级存放，既便于用户检查，也随配置一起迁移。
pub fn known_hosts_path(config_path: &Path) -> PathBuf {
    app_data_dir(config_path).join("known_hosts")
}

/// 单个隧道的诊断日志路径。
///
/// 以隧道 `id`（UUID）而非 `name` 命名。`name` 经 [`sanitize_file_stem`] 归一化后会把
/// `web api` / `web:api` / `web_api` 折叠为同一个文件名，导致不同隧道互相覆盖日志；
/// `id` 天然唯一，且只含十六进制字符与连字符，归一化不会改变它。
pub fn tunnel_log_path(config_path: &Path, tunnel_id: &str) -> PathBuf {
    app_data_dir(config_path)
        .join("logs")
        .join(format!("tunnel-{}.log", sanitize_file_stem(tunnel_id)))
}

/// 把任意字符串归一化为安全的文件名主干。
///
/// 只保留字母数字与 `-` `_`，其余一律替换为 `_`；结果为空时回退为 `unnamed`，
/// 避免产出以 `.log` 结尾的隐藏文件。
pub fn sanitize_file_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "unnamed".into()
    } else {
        cleaned
    }
}

/// 一条隧道的运行期路径集合。
///
/// 由配置文件路径与隧道 `id` 推导，覆盖 [`StartOptions`] 中除密码外的全部字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    known_hosts: PathBuf,
    log_path: PathBuf,
}

impl RuntimePaths {
    /// 由配置文件路径与隧道 `id` 推导。
    ///
    /// 传入隧道 `id` 而非名称：日志文件名以 `id` 为准（见 [`tunnel_log_path`]）。
    pub fn for_tunnel(config_path: &Path, tunnel_id: &str) -> Self {
        Self {
            known_hosts: known_hosts_path(config_path),
            log_path: tunnel_log_path(config_path, tunnel_id),
        }
    }

    /// 应用私有 `known_hosts`。调用方需要预登记 Host Key 时用它。
    pub fn known_hosts(&self) -> &Path {
        &self.known_hosts
    }

    /// 诊断日志路径。
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    /// 组装 [`StartOptions`]。
    ///
    /// 壳层应**一律**经由本方法构造选项——它是保证 `known_hosts` 与日志路径
    /// 不被遗漏的入口。`password` 由壳层自行提供（只有掌握平台解密能力的壳层才有）。
    pub fn options<'a>(&'a self, password: Option<&'a str>) -> StartOptions<'a> {
        StartOptions {
            password,
            known_hosts: Some(&self.known_hosts),
            log_path: Some(&self.log_path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_dir_falls_back_to_the_current_directory_without_a_parent() {
        assert_eq!(app_data_dir(Path::new("config.json")), PathBuf::from("."));
    }

    #[test]
    fn app_data_dir_is_the_configuration_directory() {
        assert_eq!(
            app_data_dir(Path::new("E:/app/config.json")),
            PathBuf::from("E:/app")
        );
    }

    /// **污染回归测试**：`known_hosts` 必须与配置同级，且**绝不能**落在 `~/.ssh` 下。
    ///
    /// 这正是 CLI 曾经踩的坑——传 `known_hosts: None` 时 OpenSSH 会退回
    /// `~/.ssh/known_hosts`，把应用自身的信任记录写进用户的系统文件。
    #[test]
    fn known_hosts_sits_beside_the_configuration_and_never_in_the_home_ssh_directory() {
        let path = known_hosts_path(Path::new("E:/app-data/config.json"));

        assert_eq!(path, PathBuf::from("E:/app-data/known_hosts"));
        assert!(
            !path.to_string_lossy().contains(".ssh"),
            "known_hosts 不得落在 ~/.ssh 下，实际 {}",
            path.display()
        );
    }

    #[test]
    fn tunnel_log_path_is_derived_from_the_tunnel_id() {
        let path = tunnel_log_path(Path::new("E:/app-data/config.json"), "tunnel-abc");

        assert_eq!(
            path,
            PathBuf::from("E:/app-data/logs/tunnel-tunnel-abc.log")
        );
    }

    /// 隧道名可含空格与冒号，归一化后极易撞车；`id` 是 UUID，归一化对它是恒等映射。
    #[test]
    fn sanitize_file_stem_folds_unsafe_characters_and_never_yields_an_empty_stem() {
        assert_eq!(sanitize_file_stem("web api"), "web_api");
        assert_eq!(sanitize_file_stem("web:api"), "web_api");
        assert_eq!(
            sanitize_file_stem("11111111-1111-1111-1111-111111111111"),
            "11111111-1111-1111-1111-111111111111",
            "UUID 必须被原样保留，否则不同隧道仍会撞车"
        );
        assert_eq!(sanitize_file_stem(""), "unnamed");
        assert_eq!(sanitize_file_stem("///"), "___");
    }

    #[test]
    fn for_tunnel_exposes_both_paths() {
        let paths = RuntimePaths::for_tunnel(Path::new("E:/app-data/config.json"), "abc");

        assert_eq!(paths.known_hosts(), Path::new("E:/app-data/known_hosts"));
        assert_eq!(
            paths.log_path(),
            Path::new("E:/app-data/logs/tunnel-abc.log")
        );
    }

    /// [`RuntimePaths::options`] 必须把两个路径都填上——它是防止壳层遗漏的唯一入口。
    #[test]
    fn options_always_carry_known_hosts_and_a_log_path() {
        let paths = RuntimePaths::for_tunnel(Path::new("E:/app-data/config.json"), "abc");

        let options = paths.options(Some("secret"));

        assert_eq!(options.password, Some("secret"));
        assert_eq!(
            options.known_hosts,
            Some(Path::new("E:/app-data/known_hosts"))
        );
        assert_eq!(
            options.log_path,
            Some(Path::new("E:/app-data/logs/tunnel-abc.log"))
        );
    }

    #[test]
    fn options_without_a_password_still_carry_the_paths() {
        let paths = RuntimePaths::for_tunnel(Path::new("E:/app-data/config.json"), "abc");

        let options = paths.options(None);

        assert!(options.password.is_none());
        assert!(options.known_hosts.is_some());
        assert!(options.log_path.is_some());
    }
}
