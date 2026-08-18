import { FormEvent, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { check, type Update } from "@tauri-apps/plugin-updater";

type AuthType = "ssh_agent" | "private_key" | "password";
type TunnelType = "local" | "dynamic" | "remote";
type Theme = "light" | "dark";
type ViewMode = "grid" | "table";

type Host = {
  id: string;
  name: string;
  hostname: string;
  port: number;
  username: string;
  auth: { type: AuthType; private_key?: string };
  jump_host_id?: string;
  proxy_command?: string;
  identities_only?: boolean;
  certificate_file?: string;
  compression?: boolean;
  custom_options?: string[];
  enabled: boolean;
};

type Endpoint = { host: string; port: number };

type Tunnel = {
  id: string;
  name: string;
  host_id: string;
  type: TunnelType;
  local: Endpoint;
  remote?: Endpoint;
  gateway_ports?: boolean;
  custom_options?: string[];
  auto_open_browser: boolean;
  enabled: boolean;
};

type Settings = {
  strict_host_key_checking: boolean;
  connect_timeout_seconds: number;
  server_alive_interval_seconds: number;
  server_alive_count_max: number;
  tcp_keep_alive: boolean;
  compression: boolean;
};

type Status = {
  state: "stopped" | "starting" | "running" | "error";
  message?: string;
};

type Snapshot = {
  path: string;
  version?: string;
  config: {
    settings: Settings;
    hosts: Host[];
    tunnels: Tunnel[];
  };
  statuses: Record<string, Status>;
};

type HostForm = {
  name: string;
  hostname: string;
  port: number;
  username: string;
  authType: AuthType;
  privateKey: string;
  password: string;
  jumpHostId: string;
  proxyCommand: string;
  identitiesOnly: boolean;
  certificateFile: string;
  compression: boolean;
  customOptionsText: string;
};

type TunnelForm = {
  name: string;
  hostName: string;
  kind: TunnelType;
  localHost: string;
  localPort: number;
  remoteHost: string;
  remotePort: number;
  gatewayPorts: boolean;
  autoOpenBrowser: boolean;
  customOptionsText: string;
};

type SettingsForm = {
  strictHostKeyChecking: boolean;
  connectTimeoutSeconds: number;
  serverAliveIntervalSeconds: number;
  serverAliveCountMax: number;
  tcpKeepAlive: boolean;
  compression: boolean;
};

const repositoryUrl = "https://github.com/RaInSLc/ssh_forward";

const newHost = (): HostForm => ({
  name: "",
  hostname: "",
  port: 22,
  username: "",
  authType: "password",
  privateKey: "",
  password: "",
  jumpHostId: "",
  proxyCommand: "",
  identitiesOnly: true,
  certificateFile: "",
  compression: false,
  customOptionsText: "",
});

const newTunnel = (): TunnelForm => ({
  name: "",
  hostName: "",
  kind: "local",
  localHost: "127.0.0.1",
  localPort: 18888,
  remoteHost: "127.0.0.1",
  remotePort: 8888,
  gatewayPorts: false,
  autoOpenBrowser: false,
  customOptionsText: "",
});

const newSettings = (): SettingsForm => ({
  strictHostKeyChecking: true,
  connectTimeoutSeconds: 10,
  serverAliveIntervalSeconds: 15,
  serverAliveCountMax: 3,
  tcpKeepAlive: true,
  compression: false,
});

const storedTheme = (): Theme =>
  localStorage.getItem("ssh-forward-theme") === "dark" ? "dark" : "light";

const storedAdvancedMode = (): boolean =>
  localStorage.getItem("ssh-forward-advanced-mode") === "true";

const storedViewMode = (): ViewMode =>
  localStorage.getItem("ssh-forward-view-mode") === "table" ? "table" : "grid";

const fetchAvailablePort = async (host?: string): Promise<number> => {
  try {
    return await invoke<number>("get_available_port", {
      host: host || "127.0.0.1",
    });
  } catch {
    return Math.floor(10000 + Math.random() * 50000);
  }
};

export default function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [notice, setNotice] = useState("正在加载配置...");
  const [hostForm, setHostForm] = useState<HostForm>(newHost);
  const [tunnelForm, setTunnelForm] = useState<TunnelForm>(newTunnel);
  const [settingsForm, setSettingsForm] = useState<SettingsForm>(newSettings);
  const [hostEditing, setHostEditing] = useState<string | null>(null);
  const [tunnelEditing, setTunnelEditing] = useState<string | null>(null);
  const [panel, setPanel] = useState<"host" | "tunnel" | "settings" | null>(null);
  const [showAdvancedHost, setShowAdvancedHost] = useState(false);
  const [showAdvancedTunnel, setShowAdvancedTunnel] = useState(false);
  const [formError, setFormError] = useState("");
  const [aboutOpen, setAboutOpen] = useState(false);
  const [theme, setTheme] = useState<Theme>(storedTheme);
  const [advancedMode, setAdvancedMode] = useState<boolean>(storedAdvancedMode);
  const [viewMode, setViewMode] = useState<ViewMode>(storedViewMode);
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const appVersion = snapshot?.version ?? "0.1.15";

  const [updateAvailable, setUpdateAvailable] = useState<Update | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<number | null>(null);
  const [updateDownloaded, setUpdateDownloaded] = useState(false);
  const [updateMessage, setUpdateMessage] = useState("");

  const toggleAdvancedMode = () => {
    const next = !advancedMode;
    setAdvancedMode(next);
    localStorage.setItem("ssh-forward-advanced-mode", String(next));
  };

  const toggleViewMode = () => {
    const next: ViewMode = viewMode === "grid" ? "table" : "grid";
    setViewMode(next);
    localStorage.setItem("ssh-forward-view-mode", next);
  };

  const checkForUpdates = async (manual = true) => {
    try {
      setCheckingUpdate(true);
      setUpdateMessage("正在检查更新...");
      const update = await check();
      if (update?.available) {
        setUpdateAvailable(update);
        setUpdateMessage(`发现新版本 v${update.version}`);
      } else {
        setUpdateAvailable(null);
        if (manual) {
          setUpdateMessage(`当前已是最新版本 (v${appVersion})`);
        }
      }
    } catch (error) {
      const errStr = String(error);
      if (
        errStr.includes("None of the fallback platforms") ||
        errStr.includes("404") ||
        errStr.includes("not found")
      ) {
        setUpdateAvailable(null);
        if (manual) {
          setUpdateMessage(`当前已是最新版本 (v${appVersion})`);
        }
      } else {
        console.error("检查更新失败", error);
        if (manual) {
          setUpdateMessage(`检查更新提示: ${errStr}`);
        }
      }
    } finally {
      setCheckingUpdate(false);
    }
  };

  const downloadAndInstallUpdate = async () => {
    if (!updateAvailable) return;
    try {
      setUpdateProgress(0);
      setUpdateMessage("正在下载更新...");
      let downloaded = 0;
      let contentLength = 0;
      await updateAvailable.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            contentLength = event.data.contentLength ?? 0;
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            if (contentLength > 0) {
              setUpdateProgress(Math.round((downloaded / contentLength) * 100));
            }
            break;
          case "Finished":
            setUpdateProgress(100);
            setUpdateDownloaded(true);
            setUpdateMessage("更新下载完成，准备安装并生效...");
            break;
        }
      });
      setUpdateDownloaded(true);
      setUpdateMessage("更新已安装完成，请重启应用生效。");
    } catch (error) {
      console.error("下载安装更新失败", error);
      setUpdateMessage(`更新失败: ${String(error)}`);
      setUpdateProgress(null);
    }
  };

  const loadData = async () => {
    try {
      const res = await invoke<Snapshot>("get_snapshot");
      setSnapshot(res);
      if (res.config?.settings) {
        setSettingsForm({
          strictHostKeyChecking: res.config.settings.strict_host_key_checking ?? true,
          connectTimeoutSeconds: res.config.settings.connect_timeout_seconds ?? 10,
          serverAliveIntervalSeconds: res.config.settings.server_alive_interval_seconds ?? 15,
          serverAliveCountMax: res.config.settings.server_alive_count_max ?? 3,
          tcpKeepAlive: res.config.settings.tcp_keep_alive ?? true,
          compression: res.config.settings.compression ?? false,
        });
      }
    } catch (error) {
      setNotice(String(error));
    }
  };

  useEffect(() => {
    void loadData();
    const timer = window.setInterval(() => void loadData(), 1500);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("ssh-forward-theme", theme);
  }, [theme]);

  const action = async (command: string, payload: Record<string, unknown>) => {
    try {
      await invoke(command, payload);
      setNotice("操作完成");
      await loadData();
      return "";
    } catch (error) {
      const message = String(error);
      setNotice(message);
      return message;
    }
  };

  const deleteHost = async (name: string) => {
    const host = snapshot?.config.hosts.find((item) => item.name === name);
    if (host) {
      const boundTunnels = (snapshot?.config.tunnels ?? []).filter(
        (t) => t.host_id === host.id
      );
      if (boundTunnels.length > 0) {
        setFormError(
          `无法删除服务器：仍有 ${boundTunnels.length} 个 Tunnel 关联到此服务器，请先删除对应的 Tunnel。`
        );
        return;
      }
    }
    if (!window.confirm(`确定要删除服务器“${name}”吗？`)) {
      return;
    }
    const error = await action("delete_host", { name });
    if (error) {
      setFormError(`无法删除服务器：${error}`);
    } else {
      setPanel(null);
      setHostEditing(null);
      setHostForm(newHost());
    }
  };

  const deleteTunnel = async (name: string) => {
    if (!window.confirm(`确定要删除 Tunnel“${name}”吗？`)) {
      return;
    }
    const error = await action("delete_tunnel", { name });
    if (error) {
      setFormError(`无法删除 Tunnel：${error}`);
    } else {
      setPanel(null);
      setTunnelEditing(null);
      setTunnelForm(newTunnel());
    }
  };

  const saveHost = async (event: FormEvent) => {
    event.preventDefault();
    setFormError("");
    const customOptions = hostForm.customOptionsText
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean);

    const input = {
      name: hostForm.name,
      hostname: hostForm.hostname,
      port: Number(hostForm.port),
      username: hostForm.username,
      authType: hostForm.authType,
      password: hostForm.password || null,
      privateKey: hostForm.privateKey || null,
      jumpHostId: advancedMode ? hostForm.jumpHostId || null : null,
      proxyCommand: advancedMode ? hostForm.proxyCommand || null : null,
      identitiesOnly: advancedMode ? hostForm.identitiesOnly : true,
      certificateFile: advancedMode ? hostForm.certificateFile || null : null,
      compression: advancedMode ? hostForm.compression : false,
      customOptions: advancedMode && customOptions.length ? customOptions : null,
    };
    const error = await action(
      hostEditing ? "edit_host" : "create_host",
      hostEditing ? { originalName: hostEditing, input } : { input }
    );
    if (error) {
      setFormError(`无法保存服务器：${error}`);
    } else {
      setPanel(null);
      setHostEditing(null);
      setHostForm(newHost());
    }
  };

  const saveTunnel = async (event: FormEvent) => {
    event.preventDefault();
    setFormError("");
    const customOptions = tunnelForm.customOptionsText
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean);

    const kind = advancedMode ? tunnelForm.kind : "local";
    const isDynamic = kind === "dynamic";
    const input = {
      name: tunnelForm.name,
      hostName: tunnelForm.hostName,
      kind,
      localHost: tunnelForm.localHost,
      localPort: Number(tunnelForm.localPort),
      remoteHost: isDynamic ? null : tunnelForm.remoteHost,
      remotePort: isDynamic ? null : Number(tunnelForm.remotePort),
      gatewayPorts: advancedMode ? tunnelForm.gatewayPorts : false,
      customOptions: advancedMode && customOptions.length ? customOptions : null,
      autoOpenBrowser: isDynamic ? false : tunnelForm.autoOpenBrowser,
    };
    const error = await action(
      tunnelEditing ? "edit_tunnel" : "create_tunnel",
      tunnelEditing ? { originalName: tunnelEditing, input } : { input }
    );
    if (error) {
      setFormError(`无法保存 Tunnel：${error}`);
    } else {
      setPanel(null);
      setTunnelEditing(null);
      setTunnelForm(newTunnel());
    }
  };

  const saveGlobalSettings = async (event: FormEvent) => {
    event.preventDefault();
    setFormError("");
    const input = {
      strictHostKeyChecking: settingsForm.strictHostKeyChecking,
      connectTimeoutSeconds: Number(settingsForm.connectTimeoutSeconds),
      serverAliveIntervalSeconds: Number(settingsForm.serverAliveIntervalSeconds),
      serverAliveCountMax: Number(settingsForm.serverAliveCountMax),
      tcpKeepAlive: settingsForm.tcpKeepAlive,
      compression: settingsForm.compression,
    };
    const error = await action("save_settings", { input });
    if (error) {
      setFormError(`保存设置失败：${error}`);
    } else {
      setPanel(null);
    }
  };

  const hosts = snapshot?.config.hosts ?? [];
  const tunnels = snapshot?.config.tunnels ?? [];
  const statuses = snapshot?.statuses ?? {};

  const closePanel = () => {
    setFormError("");
    setPanel(null);
  };

  const openHost = (host?: Host) => {
    setFormError("");
    setHostEditing(host?.name ?? null);
    setShowAdvancedHost(
      Boolean(
        host?.jump_host_id ||
          host?.proxy_command ||
          host?.certificate_file ||
          host?.compression ||
          host?.custom_options?.length
      )
    );
    setHostForm(
      host
        ? {
            name: host.name,
            hostname: host.hostname,
            port: host.port,
            username: host.username,
            authType: host.auth.type,
            privateKey: host.auth.private_key ?? "",
            password: "",
            jumpHostId: host.jump_host_id ?? "",
            proxyCommand: host.proxy_command ?? "",
            identitiesOnly: host.identities_only ?? true,
            certificateFile: host.certificate_file ?? "",
            compression: host.compression ?? false,
            customOptionsText: (host.custom_options ?? []).join("\n"),
          }
        : newHost()
    );
    setPanel("host");
  };

  const openTunnel = async (tunnel?: Tunnel) => {
    setFormError("");
    const host = tunnel && hosts.find((item) => item.id === tunnel.host_id);
    setTunnelEditing(tunnel?.name ?? null);
    setShowAdvancedTunnel(
      Boolean(tunnel?.gateway_ports || tunnel?.custom_options?.length)
    );
    if (tunnel) {
      setTunnelForm({
        name: tunnel.name,
        hostName: host?.name ?? "",
        kind: tunnel.type ?? "local",
        localHost: tunnel.local.host,
        localPort: tunnel.local.port,
        remoteHost: tunnel.remote?.host ?? "127.0.0.1",
        remotePort: tunnel.remote?.port ?? 8888,
        gatewayPorts: tunnel.gateway_ports ?? false,
        autoOpenBrowser: tunnel.auto_open_browser ?? false,
        customOptionsText: (tunnel.custom_options ?? []).join("\n"),
      });
    } else {
      const defaultForm = newTunnel();
      const randomPort = await fetchAvailablePort(defaultForm.localHost);
      defaultForm.localPort = randomPort;
      setTunnelForm(defaultForm);
    }
    setPanel("tunnel");
  };

  const openSettings = () => {
    setFormError("");
    setPanel("settings");
  };

  const copyToClipboard = (text: string, id: string) => {
    void navigator.clipboard.writeText(text);
    setCopiedId(id);
    setTimeout(() => setCopiedId(null), 2000);
  };

  return (
    <main>
      <aside>
        <div className="brand">
          <span>SF</span>
          <div>
            <strong>SSH Forward</strong>
            <small>{advancedMode ? "专业进阶版 (已开启高级设置)" : "本地端口转发客户端"}</small>
          </div>
        </div>

        <div className="sidebar-section-header">
          <p className="eyebrow">HOSTS / 服务器列表</p>
          <button className="icon-btn" title="添加服务器" onClick={() => openHost()}>
            +
          </button>
        </div>
        <div className="host-list">
          {hosts.map((host) => (
            <button
              className="host-row"
              key={host.id}
              onClick={() => openHost(host)}
            >
              <i></i>
              <span>
                <b>{host.name}</b>
                <small>
                  {host.username}@{host.hostname}:{host.port}
                </small>
                {advancedMode && host.jump_host_id && (
                  <span className="mini-badge">🦘 跳板机</span>
                )}
              </span>
            </button>
          ))}
          {!hosts.length && (
            <div className="empty-sidebar">点击下方按钮添加首个 SSH 服务器</div>
          )}
        </div>
        <button className="outline add-host-btn" onClick={() => openHost()}>
          + 添加服务器
        </button>

        {advancedMode && (
          <button className="settings-link-btn" onClick={openSettings}>
            ⚙️ 全局网络与保活设置
          </button>
        )}

        <footer>
          <span>{notice}</span>
          <span className="config-path" title={snapshot?.path}>
            配置文件：{snapshot?.path ?? "加载中..."}
          </span>
          <button className="about-link" onClick={() => setAboutOpen(true)}>
            关于 SSH Forward v{appVersion}
          </button>
        </footer>
      </aside>

      <section className="workspace">
        <header>
          <div>
            <p className="eyebrow">
              {advancedMode ? "FORWARDS & PROXY / 进阶转发与代理" : "LOCAL FORWARDS / 本地端口转发"}
            </p>
            <h1>转发控制台</h1>
          </div>
          <div className="header-actions">
            {/* 高级模式切换开关按钮 */}
            <button
              className={`mode-toggle-btn ${advancedMode ? "active" : ""}`}
              onClick={toggleAdvancedMode}
              title={advancedMode ? "点击切回 0.1.13 基础精简模式" : "点击开启 0.1.14 高级设置 (SOCKS5/反向穿透/跳板机/保活)"}
            >
              {advancedMode ? "⚡ 高级模式：已开启" : "⚙️ 开启高级设置"}
            </button>

            {/* 视图切换按钮 */}
            <button
              className="view-toggle-btn"
              onClick={toggleViewMode}
              title="切换视图展示模式 (卡片 / 表格)"
            >
              {viewMode === "grid" ? "📋 切换表格" : "🗂️ 切换卡片"}
            </button>

            <button
              className="theme-toggle"
              onClick={() => setTheme(theme === "light" ? "dark" : "light")}
            >
              {theme === "light" ? "🌙 夜间" : "☀️ 日间"}
            </button>
            <button
              className="primary"
              disabled={!hosts.length}
              onClick={() => openTunnel()}
            >
              {advancedMode ? "+ 新建 Tunnel / 代理" : "+ 新建 Tunnel"}
            </button>
          </div>
        </header>

        {/* 隧道列表容器：支持卡片与表格视图自适应 */}
        <div className={`tunnels-container view-${viewMode}`}>
          {/* 表格视图 Table View */}
          <div className="table-wrapper">
            <table className="tunnels-table">
              <thead>
                <tr>
                  <th>名称 / 模式</th>
                  <th>本地端口 / 地址</th>
                  <th>远端目标 / 路径</th>
                  <th>状态</th>
                  <th style={{ textAlign: "right" }}>操作</th>
                </tr>
              </thead>
              <tbody>
                {tunnels.map((tunnel) => {
                  const status = statuses[tunnel.name] ?? { state: "stopped" as const };
                  const running = status.state === "running";
                  const host = hosts.find((h) => h.id === tunnel.host_id);
                  const isDynamic = tunnel.type === "dynamic";
                  const isRemote = tunnel.type === "remote";
                  const socksAddress = `socks5://${tunnel.local.host}:${tunnel.local.port}`;
                  const localAddress = `http://${tunnel.local.host}:${tunnel.local.port}`;

                  return (
                    <tr key={tunnel.id}>
                      <td>
                        <div className="table-title-cell">
                          <strong>{tunnel.name}</strong>
                          {advancedMode && (
                            <span className={`mode-badge ${tunnel.type}`}>
                              {isDynamic
                                ? "SOCKS5 代理"
                                : isRemote
                                ? "Remote 穿透"
                                : "Local 转发"}
                            </span>
                          )}
                          {advancedMode && tunnel.gateway_ports && (
                            <span className="mode-badge gateway">共享</span>
                          )}
                        </div>
                        {status.message && (
                          <div className="table-error-hint">{status.message}</div>
                        )}
                      </td>
                      <td>
                        <code className="port-badge">
                          {tunnel.local.host}:{tunnel.local.port}
                        </code>
                      </td>
                      <td>
                        {isDynamic ? (
                          <span className="table-dest-desc">
                            🌐 代理访问 <code>{host?.hostname ?? "远端服务器"}</code> 私有网络
                          </span>
                        ) : isRemote ? (
                          <span className="table-dest-desc">
                            📡 公网 <code>{tunnel.remote?.host ?? "0.0.0.0"}:{tunnel.remote?.port}</code> → 本地
                          </span>
                        ) : (
                          <span className="table-dest-desc">
                            → <code>{tunnel.remote?.host}:{tunnel.remote?.port}</code> ({host?.name ?? ""})
                          </span>
                        )}
                      </td>
                      <td>
                        <span className={`status ${status.state}`}>
                          {running ? "运行中" : status.state === "error" ? "异常" : "已停止"}
                        </span>
                      </td>
                      <td>
                        <div className="table-actions">
                          {isDynamic ? (
                            <button
                              className="btn-sm"
                              disabled={!running}
                              onClick={() => copyToClipboard(socksAddress, `tbl-socks-${tunnel.id}`)}
                            >
                              {copiedId === `tbl-socks-${tunnel.id}` ? "✓ 已复制" : "复制代理"}
                            </button>
                          ) : isRemote ? (
                            <button
                              className="btn-sm"
                              disabled={!running}
                              onClick={() =>
                                copyToClipboard(
                                  `${host?.hostname ?? "服务器IP"}:${tunnel.remote?.port}`,
                                  `tbl-remote-${tunnel.id}`
                                )
                              }
                            >
                              {copiedId === `tbl-remote-${tunnel.id}` ? "✓ 已复制" : "复制公网"}
                            </button>
                          ) : (
                            <>
                              <button
                                className="btn-sm"
                                disabled={!running}
                                onClick={() =>
                                  void action("open_tunnel_in_browser", { name: tunnel.name })
                                }
                              >
                                打开
                              </button>
                              <button
                                className="btn-sm"
                                disabled={!running}
                                onClick={() => copyToClipboard(localAddress, `tbl-local-${tunnel.id}`)}
                              >
                                {copiedId === `tbl-local-${tunnel.id}` ? "✓ 已复制" : "复制"}
                              </button>
                            </>
                          )}
                          <button
                            className={`btn-sm ${running ? "btn-stop" : "btn-start"}`}
                            onClick={() =>
                              void action(running ? "stop_tunnel" : "start_tunnel", {
                                name: tunnel.name,
                              })
                            }
                          >
                            {running ? "停止" : "启动"}
                          </button>
                          <button className="btn-sm" onClick={() => openTunnel(tunnel)}>
                            编辑
                          </button>
                          <button
                            className="btn-sm danger-link"
                            onClick={() => void deleteTunnel(tunnel.name)}
                          >
                            删除
                          </button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>

          {/* 卡片视图 Card Grid View */}
          <div className="tunnels-grid">
            {tunnels.map((tunnel) => {
              const status = statuses[tunnel.name] ?? {
                state: "stopped" as const,
              };
              const running = status.state === "running";
              const host = hosts.find((h) => h.id === tunnel.host_id);
              const isDynamic = tunnel.type === "dynamic";
              const isRemote = tunnel.type === "remote";
              const socksAddress = `socks5://${tunnel.local.host}:${tunnel.local.port}`;
              const localAddress = `http://${tunnel.local.host}:${tunnel.local.port}`;

              return (
                <article key={tunnel.id} className={`tunnel-card ${tunnel.type}`}>
                  <div className="tunnel-head">
                    <div>
                      <div className="tunnel-title-row">
                        <h2>{tunnel.name}</h2>
                        {advancedMode && (
                          <span className={`mode-badge ${tunnel.type}`}>
                            {isDynamic
                              ? "SOCKS5 动态代理"
                              : isRemote
                              ? "Remote 反向穿透"
                              : "Local 本地转发"}
                          </span>
                        )}
                        {advancedMode && tunnel.gateway_ports && (
                          <span className="mode-badge gateway">局域网共享</span>
                        )}
                      </div>

                      <div className="tunnel-route">
                        {isDynamic ? (
                          <div className="route-desc">
                            <span>代理端口：</span>
                            <strong>{tunnel.local.host}:{tunnel.local.port}</strong>
                            <span className="route-arrow">→</span>
                            <span className="route-dest">
                              <code>{host?.hostname ?? "远端服务器"}</code> 全网
                            </span>
                          </div>
                        ) : isRemote ? (
                          <div className="route-desc">
                            <span>公网：</span>
                            <strong>{tunnel.remote?.host ?? "0.0.0.0"}:{tunnel.remote?.port}</strong>
                            <span className="route-arrow">→</span>
                            <span>本地：</span>
                            <strong>{tunnel.local.host}:{tunnel.local.port}</strong>
                          </div>
                        ) : (
                          <div className="route-desc">
                            <span>本地：</span>
                            <strong>{tunnel.local.host}:{tunnel.local.port}</strong>
                            <span className="route-arrow">→</span>
                            <span>远端：</span>
                            <strong>{tunnel.remote?.host}:{tunnel.remote?.port}</strong>
                          </div>
                        )}
                      </div>
                    </div>

                    <span className={`status ${status.state}`}>
                      {running
                        ? "运行中"
                        : status.state === "error"
                        ? "异常"
                        : "已停止"}
                    </span>
                  </div>

                  {status.message && (
                    <div className="error-box">
                      <p className="error-text">{status.message}</p>
                      {host && (
                        <button
                          type="button"
                          className="error-action-btn"
                          onClick={() => openHost(host)}
                        >
                          ⚙️ 去修改服务器“{host.name}”的设置
                        </button>
                      )}
                    </div>
                  )}

                  <div className="actions">
                    {isDynamic ? (
                      <button
                        disabled={!running}
                        onClick={() => copyToClipboard(socksAddress, `socks-${tunnel.id}`)}
                      >
                        {copiedId === `socks-${tunnel.id}` ? "✓ 已复制" : "复制代理"}
                      </button>
                    ) : isRemote ? (
                      <button
                        disabled={!running}
                        onClick={() =>
                          copyToClipboard(
                            `${host?.hostname ?? "服务器IP"}:${tunnel.remote?.port}`,
                            `remote-${tunnel.id}`
                          )
                        }
                      >
                        {copiedId === `remote-${tunnel.id}` ? "✓ 已复制" : "复制公网"}
                      </button>
                    ) : (
                      <>
                        <button
                          disabled={!running}
                          onClick={() =>
                            void action("open_tunnel_in_browser", {
                              name: tunnel.name,
                            })
                          }
                        >
                          打开浏览器
                        </button>
                        <button
                          disabled={!running}
                          onClick={() => copyToClipboard(localAddress, `local-${tunnel.id}`)}
                        >
                          {copiedId === `local-${tunnel.id}` ? "✓ 已复制" : "复制地址"}
                        </button>
                      </>
                    )}

                    <button
                      className={running ? "btn-stop" : "btn-start"}
                      onClick={() =>
                        void action(running ? "stop_tunnel" : "start_tunnel", {
                          name: tunnel.name,
                        })
                      }
                    >
                      {running ? "停止" : "启动"}
                    </button>
                    <button onClick={() => openTunnel(tunnel)}>编辑</button>
                    <button
                      className="danger-link"
                      onClick={() => void deleteTunnel(tunnel.name)}
                    >
                      删除
                    </button>
                  </div>
                </article>
              );
            })}
          </div>

          {!tunnels.length && (
            <div className="zero">
              <h2>还没有创建 Tunnel</h2>
              <p>点击上方“+ 新建 Tunnel”，建立首条本地端口转发。</p>
            </div>
          )}
        </div>
      </section>

      {/* 服务器设置弹窗 */}
      {panel === "host" && (
        <Modal
          title={hostEditing ? "编辑服务器" : "添加服务器"}
          close={closePanel}
        >
          <form onSubmit={saveHost}>
            <Field
              label="名称 (用于区分识别)"
              value={hostForm.name}
              set={(value) => setHostForm({ ...hostForm, name: value })}
            />
            <Field
              label="服务器主机地址 (IP 或域名)"
              value={hostForm.hostname}
              set={(value) => setHostForm({ ...hostForm, hostname: value })}
            />
            <div className="pair">
              <Field
                label="SSH 端口"
                value={hostForm.port}
                type="number"
                set={(value) =>
                  setHostForm({ ...hostForm, port: Number(value) })
                }
              />
              <Field
                label="SSH 用户名"
                value={hostForm.username}
                set={(value) => setHostForm({ ...hostForm, username: value })}
              />
            </div>
            <label>
              认证方式
              <select
                value={hostForm.authType}
                onChange={(event) =>
                  setHostForm({
                    ...hostForm,
                    authType: event.target.value as AuthType,
                  })
                }
              >
                <option value="password">密码 (推荐常规使用)</option>
                <option value="private_key">私钥文件 (.pem / id_rsa / id_ed25519)</option>
                <option value="ssh_agent">SSH Agent (系统后台密钥代理)</option>
              </select>
            </label>

            {hostForm.authType === "ssh_agent" && (
              <div className="auth-hint-card">
                <p>
                  ℹ️ <strong>使用说明：</strong>此模式直接复用系统加载的 SSH 私钥，无需在本软件输入密码。
                </p>
                <p>
                  需确保本机已运行 <code>ssh-agent</code> 服务并执行过 <code>ssh-add</code>。
                </p>
              </div>
            )}
            {hostForm.authType === "private_key" && (
              <div>
                <Field
                  label="私钥路径"
                  value={hostForm.privateKey}
                  set={(value) => setHostForm({ ...hostForm, privateKey: value })}
                />
                <small className="field-hint">
                  例如：C:\Users\Admin\.ssh\id_rsa 或 /Users/name/.ssh/id_ed25519
                </small>
              </div>
            )}
            {hostForm.authType === "password" && (
              <div>
                <Field
                  label="登录密码"
                  type="password"
                  value={hostForm.password}
                  set={(value) => setHostForm({ ...hostForm, password: value })}
                />
                <small className="field-hint">🔒 密码将通过系统本地安全凭据（Windows DPAPI）加密存储</small>
              </div>
            )}

            {/* 高级模式下才显示服务器高级折叠面板 */}
            {advancedMode && (
              <div className="accordion-section">
                <button
                  type="button"
                  className="accordion-toggle"
                  onClick={() => setShowAdvancedHost(!showAdvancedHost)}
                >
                  <span>{showAdvancedHost ? "▼" : "▶"} 高级设置（跳板机、证书、网络优化与自定义参数）</span>
                </button>

                {showAdvancedHost && (
                  <div className="accordion-content">
                    <label>
                      跳板机 / 堡垒机 (ProxyJump)
                      <select
                        value={hostForm.jumpHostId}
                        onChange={(e) =>
                          setHostForm({ ...hostForm, jumpHostId: e.target.value })
                        }
                      >
                        <option value="">无（直连此服务器）</option>
                        {hosts
                          .filter((h) => !hostEditing || h.name !== hostEditing)
                          .map((h) => (
                            <option key={h.id} value={h.id}>
                              {h.name} ({h.username}@{h.hostname}:{h.port})
                            </option>
                          ))}
                      </select>
                    </label>
                    <small className="field-hint">自动追加 <code>-J jump_host</code> 通过堡垒机建立多跳安全隧道</small>

                    <Field
                      label="前置代理命令 (ProxyCommand，可选)"
                      value={hostForm.proxyCommand}
                      set={(value) => setHostForm({ ...hostForm, proxyCommand: value })}
                    />
                    <small className="field-hint">例如通过内网 HTTP/SOCKS 代理穿透连接：<code>connect-proxy -S 127.0.0.1:1080 %h %p</code></small>

                    <Field
                      label="SSH 证书文件路径 (CertificateFile，可选)"
                      value={hostForm.certificateFile}
                      set={(value) => setHostForm({ ...hostForm, certificateFile: value })}
                    />

                    <div className="pair-checks">
                      <label className="check">
                        <input
                          type="checkbox"
                          checked={hostForm.identitiesOnly}
                          onChange={(e) =>
                            setHostForm({ ...hostForm, identitiesOnly: e.target.checked })
                          }
                        />
                        严格只用指定私钥 (IdentitiesOnly，防 Agent 密钥过多被拒)
                      </label>

                      <label className="check">
                        <input
                          type="checkbox"
                          checked={hostForm.compression}
                          onChange={(e) =>
                            setHostForm({ ...hostForm, compression: e.target.checked })
                          }
                        />
                        启用数据流压缩 (-C / Compression，优化弱网高延迟)
                      </label>
                    </div>

                    <label>
                      自定义 OpenSSH 参数 (-o 键值对，每行一个)
                      <textarea
                        rows={3}
                        value={hostForm.customOptionsText}
                        placeholder="PubkeyAcceptedKeyTypes=+ssh-rsa&#10;IPQoS=throughput"
                        onChange={(e) =>
                          setHostForm({ ...hostForm, customOptionsText: e.target.value })
                        }
                      />
                    </label>
                  </div>
                )}
              </div>
            )}

            {formError && (
              <p className="form-error" role="alert">
                {formError}
              </p>
            )}
            <div className="form-actions">
              {hostEditing && (
                <button
                  type="button"
                  className="danger"
                  onClick={() => void deleteHost(hostEditing)}
                >
                  删除服务器
                </button>
              )}
              <button
                className="primary submit"
                style={{ marginLeft: hostEditing ? "0" : "auto" }}
              >
                保存服务器
              </button>
            </div>
          </form>
        </Modal>
      )}

      {/* 隧道设置弹窗 */}
      {panel === "tunnel" && (
        <Modal
          title={tunnelEditing ? "编辑 Tunnel" : "新建 Tunnel"}
          close={closePanel}
        >
          <form onSubmit={saveTunnel}>
            {/* 高级模式下才显示转发模式分段选择器 */}
            {advancedMode && (
              <>
                <label>转发模式</label>
                <div className="mode-selector">
                  <button
                    type="button"
                    className={`mode-btn ${tunnelForm.kind === "local" ? "active" : ""}`}
                    onClick={() => setTunnelForm({ ...tunnelForm, kind: "local" })}
                  >
                    <strong>🔄 本地端口转发 (-L)</strong>
                    <small>将远端内网端口映射到本机</small>
                  </button>
                  <button
                    type="button"
                    className={`mode-btn ${tunnelForm.kind === "dynamic" ? "active" : ""}`}
                    onClick={() => setTunnelForm({ ...tunnelForm, kind: "dynamic" })}
                  >
                    <strong>🌐 SOCKS5 代理 (-D)</strong>
                    <small>开放全功能动态代理网关</small>
                  </button>
                  <button
                    type="button"
                    className={`mode-btn ${tunnelForm.kind === "remote" ? "active" : ""}`}
                    onClick={() => setTunnelForm({ ...tunnelForm, kind: "remote" })}
                  >
                    <strong>📡 远程反向转发 (-R)</strong>
                    <small>内网穿透：将本机服务暴露到公网</small>
                  </button>
                </div>
              </>
            )}

            <Field
              label="名称 (用于标记区分)"
              value={tunnelForm.name}
              set={(value) => setTunnelForm({ ...tunnelForm, name: value })}
            />

            <label>
              连接目标服务器
              <select
                value={tunnelForm.hostName}
                required
                onChange={(event) =>
                  setTunnelForm({ ...tunnelForm, hostName: event.target.value })
                }
              >
                <option value="" disabled>
                  选择服务器
                </option>
                {hosts.map((host) => (
                  <option key={host.id} value={host.name}>
                    {host.name} ({host.username}@{host.hostname})
                  </option>
                ))}
              </select>
            </label>

            {/* 本地端配置 */}
            <div className="pair">
              <label>
                本地监听地址
                <select
                  value={tunnelForm.localHost}
                  onChange={(event) =>
                    setTunnelForm({ ...tunnelForm, localHost: event.target.value })
                  }
                >
                  <option value="127.0.0.1">127.0.0.1 (仅本机回环访问)</option>
                  <option value="0.0.0.0">0.0.0.0 (允许局域网设备共享)</option>
                  <option value="localhost">localhost</option>
                </select>
              </label>
              <div className="field-with-button">
                <Field
                  label={
                    advancedMode && tunnelForm.kind === "dynamic"
                      ? "本地 SOCKS5 监听端口"
                      : advancedMode && tunnelForm.kind === "remote"
                      ? "本地服务端口"
                      : "本地监听端口"
                  }
                  type="number"
                  value={tunnelForm.localPort}
                  set={(value) =>
                    setTunnelForm({ ...tunnelForm, localPort: Number(value) })
                  }
                />
                <button
                  type="button"
                  className="inline-random-btn"
                  title="随机分配一个空闲端口"
                  onClick={async () => {
                    const port = await fetchAvailablePort(tunnelForm.localHost);
                    setTunnelForm((prev) => ({ ...prev, localPort: port }));
                  }}
                >
                  🎲 随机
                </button>
              </div>
            </div>

            {/* 远端配置 */}
            {advancedMode && tunnelForm.kind === "dynamic" ? (
              <div className="auth-hint-card">
                <p>
                  💡 <strong>SOCKS5 动态代理使用提示：</strong>
                </p>
                <p>
                  启动后，可在浏览器（如 SwitchyOmega）或终端配置 SOCKS5 代理：
                  <br />
                  <code>socks5://{tunnelForm.localHost}:{tunnelForm.localPort}</code>
                  ，即可自动通过远端服务器畅通访问远程网络的所有服务与网页。
                </p>
              </div>
            ) : (
              <div className="pair">
                <Field
                  label={
                    advancedMode && tunnelForm.kind === "remote"
                      ? "远程绑定地址 (通常 0.0.0.0 或 127.0.0.1)"
                      : "远端目标主机"
                  }
                  value={tunnelForm.remoteHost}
                  set={(value) =>
                    setTunnelForm({ ...tunnelForm, remoteHost: value })
                  }
                />
                <Field
                  label={
                    advancedMode && tunnelForm.kind === "remote"
                      ? "远程公网端口 (外部访问此端口)"
                      : "远端目标端口"
                  }
                  type="number"
                  value={tunnelForm.remotePort}
                  set={(value) =>
                    setTunnelForm({ ...tunnelForm, remotePort: Number(value) })
                  }
                />
              </div>
            )}

            {(!advancedMode || tunnelForm.kind === "local") && (
              <label className="check">
                <input
                  type="checkbox"
                  checked={tunnelForm.autoOpenBrowser}
                  onChange={(event) =>
                    setTunnelForm({
                      ...tunnelForm,
                      autoOpenBrowser: event.target.checked,
                    })
                  }
                />
                启动成功后自动打开系统默认浏览器
              </label>
            )}

            {/* 高级模式下才显示隧道高级参数折叠栏 */}
            {advancedMode && (
              <div className="accordion-section">
                <button
                  type="button"
                  className="accordion-toggle"
                  onClick={() => setShowAdvancedTunnel(!showAdvancedTunnel)}
                >
                  <span>{showAdvancedTunnel ? "▼" : "▶"} 隧道高级参数（局域网网关共享、自定义 -o）</span>
                </button>
                {showAdvancedTunnel && (
                  <div className="accordion-content">
                    <label className="check">
                      <input
                        type="checkbox"
                        checked={tunnelForm.gatewayPorts}
                        onChange={(e) =>
                          setTunnelForm({
                            ...tunnelForm,
                            gatewayPorts: e.target.checked,
                            localHost: e.target.checked ? "0.0.0.0" : tunnelForm.localHost,
                          })
                        }
                      />
                      开启网关端口转发 (-g / GatewayPorts，允许局域网同伴机器连接)
                    </label>

                    <label>
                      自定义 OpenSSH 选项 (-o 参数，每行一条)
                      <textarea
                        rows={2}
                        value={tunnelForm.customOptionsText}
                        placeholder="ExitOnForwardFailure=yes"
                        onChange={(e) =>
                          setTunnelForm({
                            ...tunnelForm,
                            customOptionsText: e.target.value,
                          })
                        }
                      />
                    </label>
                  </div>
                )}
              </div>
            )}

            {formError && (
              <p className="form-error" role="alert">
                {formError}
              </p>
            )}
            <div className="form-actions">
              {tunnelEditing && (
                <button
                  type="button"
                  className="danger"
                  onClick={() => void deleteTunnel(tunnelEditing)}
                >
                  删除 Tunnel
                </button>
              )}
              <button
                className="primary submit"
                style={{ marginLeft: tunnelEditing ? "0" : "auto" }}
              >
                保存 Tunnel
              </button>
            </div>
          </form>
        </Modal>
      )}

      {/* 全局设置弹窗 */}
      {panel === "settings" && (
        <Modal title="⚙️ 全局网络与连接保活设置" close={closePanel}>
          <form onSubmit={saveGlobalSettings}>
            <div className="pair">
              <Field
                label="心跳探针周期 (ServerAliveInterval，秒)"
                type="number"
                value={settingsForm.serverAliveIntervalSeconds}
                set={(value) =>
                  setSettingsForm({
                    ...settingsForm,
                    serverAliveIntervalSeconds: Number(value),
                  })
                }
              />
              <Field
                label="探针最大未响应次数 (ServerAliveCountMax)"
                type="number"
                value={settingsForm.serverAliveCountMax}
                set={(value) =>
                  setSettingsForm({
                    ...settingsForm,
                    serverAliveCountMax: Number(value),
                  })
                }
              />
            </div>
            <small className="field-hint" style={{ marginBottom: "12px" }}>
              💡 每隔 N 秒发送探针保持连接活跃，连续未响应则判定断开并触发自动重连，彻底根治 NAT/防火墙静默丢包假死问题。
            </small>

            <div className="pair">
              <Field
                label="连接超时时间 (ConnectTimeout，秒)"
                type="number"
                value={settingsForm.connectTimeoutSeconds}
                set={(value) =>
                  setSettingsForm({
                    ...settingsForm,
                    connectTimeoutSeconds: Number(value),
                  })
                }
              />
            </div>

            <div className="pair-checks" style={{ marginTop: "8px" }}>
              <label className="check">
                <input
                  type="checkbox"
                  checked={settingsForm.tcpKeepAlive}
                  onChange={(e) =>
                    setSettingsForm({
                      ...settingsForm,
                      tcpKeepAlive: e.target.checked,
                    })
                  }
                />
                启用系统 TCP 层保活 (TCPKeepAlive)
              </label>

              <label className="check">
                <input
                  type="checkbox"
                  checked={settingsForm.strictHostKeyChecking}
                  onChange={(e) =>
                    setSettingsForm({
                      ...settingsForm,
                      strictHostKeyChecking: e.target.checked,
                    })
                  }
                />
                严格主机密钥检查 (StrictHostKeyChecking=accept-new)
              </label>

              <label className="check">
                <input
                  type="checkbox"
                  checked={settingsForm.compression}
                  onChange={(e) =>
                    setSettingsForm({
                      ...settingsForm,
                      compression: e.target.checked,
                    })
                  }
                />
                全局开启数据流压缩 (-C / Compression)
              </label>
            </div>

            {formError && (
              <p className="form-error" role="alert">
                {formError}
              </p>
            )}

            <div className="form-actions">
              <button className="primary submit" style={{ marginLeft: "auto" }}>
                保存全局设置
              </button>
            </div>
          </form>
        </Modal>
      )}

      {/* 关于弹窗 */}
      {aboutOpen && (
        <Modal title="关于 SSH Forward" close={() => setAboutOpen(false)}>
          <div className="about">
            <p style={{ fontSize: "16px", fontWeight: "bold" }}>
              SSH Forward v{appVersion}
            </p>
            <p>基于 OpenSSH 的本地端口转发、动态 SOCKS5 代理与内网穿透客户端。</p>
            <a href={repositoryUrl} target="_blank" rel="noreferrer">
              {repositoryUrl}
            </a>

            <div className="update-box">
              <div className="update-status">
                {updateMessage || `当前已安装版本: v${appVersion}`}
              </div>
              {updateProgress !== null && (
                <div className="update-progress-track">
                  <div
                    className="update-progress-fill"
                    style={{ width: `${updateProgress}%` }}
                  ></div>
                </div>
              )}
              {updateAvailable?.body && (
                <p style={{ fontSize: "12px", marginTop: "6px", whiteSpace: "pre-wrap" }}>
                  <strong>更新说明：</strong>
                  <br />
                  {updateAvailable.body}
                </p>
              )}
              <div className="update-actions">
                {updateAvailable && !updateDownloaded ? (
                  <button
                    type="button"
                    className="update-btn"
                    disabled={updateProgress !== null}
                    onClick={() => void downloadAndInstallUpdate()}
                  >
                    {updateProgress !== null ? `下载中 ${updateProgress}%` : "立即更新并安装"}
                  </button>
                ) : (
                  <button
                    type="button"
                    className="update-btn secondary"
                    disabled={checkingUpdate}
                    onClick={() => void checkForUpdates(true)}
                  >
                    {checkingUpdate ? "正在检查..." : "检查更新"}
                  </button>
                )}
              </div>
            </div>
          </div>
        </Modal>
      )}
    </main>
  );
}

function Field({
  label,
  value,
  set,
  type = "text",
}: {
  label: string;
  value: string | number;
  set: (value: string) => void;
  type?: string;
}) {
  return (
    <label>
      {label}
      <input
        required={type !== "password"}
        type={type}
        value={value}
        onChange={(event) => set(event.target.value)}
      />
    </label>
  );
}

function Modal({
  title,
  close,
  children,
}: {
  title: string;
  close: () => void;
  children: ReactNode;
}) {
  return (
    <div className="modal-bg">
      <div className="modal">
        <header>
          <h2>{title}</h2>
          <button type="button" onClick={close}>
            ×
          </button>
        </header>
        {children}
      </div>
    </div>
  );
}
