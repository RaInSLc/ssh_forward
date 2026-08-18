import { FormEvent, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { check, type Update } from "@tauri-apps/plugin-updater";

type AuthType = "ssh_agent" | "private_key" | "password";
type Theme = "light" | "dark";
type Host = {
  id: string;
  name: string;
  hostname: string;
  port: number;
  username: string;
  auth: { type: AuthType; private_key?: string };
  enabled: boolean;
};
type Endpoint = { host: string; port: number };
type Tunnel = {
  id: string;
  name: string;
  host_id: string;
  type: "local";
  local: Endpoint;
  remote: Endpoint;
  auto_open_browser: boolean;
  enabled: boolean;
};
type Status = {
  state: "stopped" | "starting" | "running" | "error";
  message?: string;
};
type Snapshot = {
  path: string;
  version?: string;
  config: { hosts: Host[]; tunnels: Tunnel[] };
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
};
type TunnelForm = {
  name: string;
  hostName: string;
  localHost: string;
  localPort: number;
  remoteHost: string;
  remotePort: number;
  autoOpenBrowser: boolean;
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
});
const newTunnel = (): TunnelForm => ({
  name: "",
  hostName: "",
  localHost: "127.0.0.1",
  localPort: 18888,
  remoteHost: "127.0.0.1",
  remotePort: 8888,
  autoOpenBrowser: false,
});
const storedTheme = (): Theme =>
  localStorage.getItem("ssh-forward-theme") === "dark" ? "dark" : "light";

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
  const [hostEditing, setHostEditing] = useState<string | null>(null);
  const [tunnelEditing, setTunnelEditing] = useState<string | null>(null);
  const [panel, setPanel] = useState<"host" | "tunnel" | null>(null);
  const [formError, setFormError] = useState("");
  const [aboutOpen, setAboutOpen] = useState(false);
  const [theme, setTheme] = useState<Theme>(storedTheme);

  const appVersion = snapshot?.version ?? "0.1.11";

  const [updateAvailable, setUpdateAvailable] = useState<Update | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<number | null>(null);
  const [updateDownloaded, setUpdateDownloaded] = useState(false);
  const [updateMessage, setUpdateMessage] = useState("");

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
        setUpdateMessage(`当前已是最新版本 (v${appVersion})`);
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

  const load = async () => {
    try {
      setSnapshot(await invoke<Snapshot>("get_snapshot"));
    } catch (error) {
      setNotice(String(error));
    }
  };
  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), 1500);
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
      await load();
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
    const input = {
      ...hostForm,
      port: Number(hostForm.port),
      password: hostForm.password || null,
      privateKey: hostForm.privateKey || null,
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
    const input = {
      ...tunnelForm,
      localPort: Number(tunnelForm.localPort),
      remotePort: Number(tunnelForm.remotePort),
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
          }
        : newHost()
    );
    setPanel("host");
  };
  const openTunnel = async (tunnel?: Tunnel) => {
    setFormError("");
    const host = tunnel && hosts.find((item) => item.id === tunnel.host_id);
    setTunnelEditing(tunnel?.name ?? null);
    if (tunnel) {
      setTunnelForm({
        name: tunnel.name,
        hostName: host?.name ?? "",
        localHost: tunnel.local.host,
        localPort: tunnel.local.port,
        remoteHost: tunnel.remote.host,
        remotePort: tunnel.remote.port,
        autoOpenBrowser: tunnel.auto_open_browser,
      });
    } else {
      const defaultForm = newTunnel();
      const randomPort = await fetchAvailablePort(defaultForm.localHost);
      defaultForm.localPort = randomPort;
      setTunnelForm(defaultForm);
    }
    setPanel("tunnel");
  };
  return (
    <main>
      <aside>
        <div className="brand">
          <span>SF</span>
          <div>
            <strong>SSH Forward</strong>
            <small>本地端口转发</small>
          </div>
        </div>
        <p className="eyebrow">HOSTS</p>
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
                  {host.username}@{host.hostname}
                </small>
              </span>
            </button>
          ))}
        </div>
        <button className="outline" onClick={() => openHost()}>
          + 添加服务器
        </button>
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
            <p className="eyebrow">LOCAL FORWARDS</p>
            <h1>转发控制台</h1>
          </div>
          <div className="header-actions">
            <button
              className="theme-toggle"
              onClick={() => setTheme(theme === "light" ? "dark" : "light")}
            >
              {theme === "light" ? "夜间模式" : "日间模式"}
            </button>
            <button
              className="primary"
              disabled={!hosts.length}
              onClick={() => openTunnel()}
            >
              + 新建 Tunnel
            </button>
          </div>
        </header>
        <div className="tunnels">
          {tunnels.map((tunnel) => {
            const status = statuses[tunnel.name] ?? {
              state: "stopped" as const,
            };
            const running = status.state === "running";
            return (
              <article key={tunnel.id}>
                <div className="tunnel-head">
                  <div>
                    <h2>{tunnel.name}</h2>
                    <p>
                      {tunnel.local.host}:{tunnel.local.port} <span>→</span>{" "}
                      {tunnel.remote.host}:{tunnel.remote.port}
                    </p>
                  </div>
                  <span className={`status ${status.state}`}>
                    {running
                      ? "运行中"
                      : status.state === "error"
                        ? "错误"
                        : "已停止"}
                  </span>
                </div>
                {status.message && (
                  <div className="error-box">
                    <p className="error-text">{status.message}</p>
                    {(() => {
                      const host = hosts.find((h) => h.id === tunnel.host_id);
                      return host ? (
                        <button
                          type="button"
                          className="error-action-btn"
                          onClick={() => openHost(host)}
                        >
                          ⚙️ 去修改服务器“{host.name}”的认证方式
                        </button>
                      ) : null;
                    })()}
                  </div>
                )}
                <div className="actions">
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

          {!tunnels.length && (
            <div className="zero">
              <h2>还没有 Tunnel</h2>
              <p>添加服务器后，新建一条本地转发。</p>
            </div>
          )}
        </div>
      </section>
      {panel === "host" && (
        <Modal
          title={hostEditing ? "编辑服务器" : "添加服务器"}
          close={closePanel}
        >
          <form onSubmit={saveHost}>
            <Field
              label="名称"
              value={hostForm.name}
              set={(value) => setHostForm({ ...hostForm, name: value })}
            />
            <Field
              label="服务器地址"
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
                label="用户名"
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
                <option value="private_key">私钥文件 (.pem / id_rsa)</option>
                <option value="ssh_agent">SSH Agent (系统后台密钥代理)</option>
              </select>
            </label>
            {hostForm.authType === "ssh_agent" && (
              <div className="auth-hint-card">
                <p>
                  ℹ️ <strong>使用前置说明：</strong>此模式直接复用系统已加载的 SSH 私钥，无需在本软件保存密码。
                </p>
                <p>
                  需确保本机已运行 <code>ssh-agent</code> 服务并执行过 <code>ssh-add</code>；若未配置请选择<strong>【密码】</strong>或<strong>【私钥】</strong>。
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
                  例如：C:\Users\Administrator\.ssh\id_rsa 或 /Users/name/.ssh/id_ed25519
                </small>
              </div>
            )}
            {hostForm.authType === "password" && (
              <div>
                <Field
                  label="密码"
                  type="password"
                  value={hostForm.password}
                  set={(value) => setHostForm({ ...hostForm, password: value })}
                />
                <small className="field-hint">🔒 密码将通过系统本地安全凭据（Windows DPAPI）加密存储</small>
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
                保存
              </button>
            </div>
          </form>
        </Modal>
      )}
      {panel === "tunnel" && (
        <Modal
          title={tunnelEditing ? "编辑 Tunnel" : "新建 Tunnel"}
          close={closePanel}
        >
          <form onSubmit={saveTunnel}>
            <Field
              label="名称"
              value={tunnelForm.name}
              set={(value) => setTunnelForm({ ...tunnelForm, name: value })}
            />
            <label>
              服务器
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
                    {host.name}
                  </option>
                ))}
              </select>
            </label>
            <div className="pair">
              <label>
                本地地址
                <select
                  value={tunnelForm.localHost}
                  onChange={(event) =>
                    setTunnelForm({ ...tunnelForm, localHost: event.target.value })
                  }
                >
                  <option value="127.0.0.1">127.0.0.1 (仅本机)</option>
                  <option value="0.0.0.0">0.0.0.0 (允许局域网访问)</option>
                  <option value="localhost">localhost</option>
                </select>
              </label>
              <div className="field-with-button">
                <Field
                  label="本地端口"
                  type="number"
                  value={tunnelForm.localPort}
                  set={(value) =>
                    setTunnelForm({ ...tunnelForm, localPort: Number(value) })
                  }
                />
                <button
                  type="button"
                  className="inline-random-btn"
                  title="随机分配一个未被占用的空闲端口"
                  onClick={async () => {
                    const port = await fetchAvailablePort(tunnelForm.localHost);
                    setTunnelForm((prev) => ({ ...prev, localPort: port }));
                  }}
                >
                  🎲 随机
                </button>
              </div>
            </div>
            <div className="pair">
              <Field
                label="远端地址"
                value={tunnelForm.remoteHost}
                set={(value) =>
                  setTunnelForm({ ...tunnelForm, remoteHost: value })
                }
              />
              <Field
                label="远端端口"
                type="number"
                value={tunnelForm.remotePort}
                set={(value) =>
                  setTunnelForm({ ...tunnelForm, remotePort: Number(value) })
                }
              />
            </div>
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
              启动成功后自动打开浏览器
            </label>
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
                保存
              </button>
            </div>
          </form>
        </Modal>
      )}
      {aboutOpen && (
        <Modal title="关于 SSH Forward" close={() => setAboutOpen(false)}>
          <div className="about">
            <p>SSH Forward v{appVersion}</p>
            <p>本地 SSH 端口转发工具。</p>
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
