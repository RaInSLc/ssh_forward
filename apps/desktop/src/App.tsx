import { FormEvent, useEffect, useId, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { check, type Update } from "@tauri-apps/plugin-updater";

type AuthType = "ssh_agent" | "private_key" | "password";
type TunnelType = "local" | "dynamic" | "remote";
type Theme = "light" | "dark";
type ViewMode = "grid" | "table";
type HostKeyPolicy = "accept_new" | "strict" | "insecure";

type Host = {
  id: string;
  name: string;
  hostname: string;
  port: number;
  username: string;
  /**
   * 与 `ssh-forward-config` 的 `Auth` 对齐。后两个字段由桌面壳层在保存密码时写入，
   * 界面不读它们，但类型必须如实描述线格式，否则新增读取方会拿到「不存在」的错觉。
   */
  auth: {
    type: AuthType;
    private_key?: string;
    credential_id?: string;
    encrypted_password?: string;
  };
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
  /** 与 Rust `Tunnel` 的 `auto_start` / `auto_reconnect` 对齐（两者目前均无实现，见 D1）。 */
  auto_start: boolean;
  auto_reconnect: boolean;
  auto_open_browser: boolean;
  enabled: boolean;
};

type Settings = {
  host_key_policy: HostKeyPolicy;
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
  platform?: string;
  supportsPasswordAuth?: boolean;
  knownHostsPath?: string;
  config: {
    settings: Settings;
    hosts: Host[];
    tunnels: Tunnel[];
  };
  statuses: Record<string, Status>;
};

/** 转发列表的分组单元：一个分组对应一台服务器（或"未关联"兜底分组）。 */
type TunnelGroup = {
  key: string;
  name: string;
  caption: string;
  tunnels: Tunnel[];
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
  hostKeyPolicy: HostKeyPolicy;
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
  hostKeyPolicy: "accept_new",
  connectTimeoutSeconds: 10,
  serverAliveIntervalSeconds: 15,
  serverAliveCountMax: 3,
  tcpKeepAlive: true,
  compression: false,
});

/**
 * 后端 settings 快照 → 设置表单值。
 *
 * 抽成模块级纯函数，使「快照映射」与「何时允许覆盖表单」两件事解耦：
 * 前者可独立验证，后者由 `settingsDirty` 单一控制。
 */
const settingsFromSnapshot = (settings: Settings): SettingsForm => ({
  hostKeyPolicy: settings.host_key_policy ?? "accept_new",
  connectTimeoutSeconds: settings.connect_timeout_seconds ?? 10,
  serverAliveIntervalSeconds: settings.server_alive_interval_seconds ?? 15,
  serverAliveCountMax: settings.server_alive_count_max ?? 3,
  tcpKeepAlive: settings.tcp_keep_alive ?? true,
  compression: settings.compression ?? false,
});

const hostKeyPolicyLabels: Record<HostKeyPolicy, string> = {
  accept_new: "首次连接自动登记，密钥变更则拒绝（推荐）",
  strict: "仅接受已知主机，未知主机直接拒绝",
  insecure: "不校验主机密钥（存在中间人风险）",
};

const storedTheme = (): Theme =>
  localStorage.getItem("ssh-forward-theme") === "dark" ? "dark" : "light";

const storedAdvancedMode = (): boolean =>
  localStorage.getItem("ssh-forward-advanced-mode") === "true";

const storedViewMode = (): ViewMode =>
  localStorage.getItem("ssh-forward-view-mode") === "table" ? "table" : "grid";

/** 分组默认开启：多服务器场景下平铺列表难以辨认归属。显式关闭后才持久化为 "false"。 */
const storedGroupByHost = (): boolean =>
  localStorage.getItem("ssh-forward-group-by-host") !== "false";

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
  const [groupByHost, setGroupByHost] = useState<boolean>(storedGroupByHost);
  // 折叠状态只存在内存中，不持久化：避免下次启动时"内容不见了"的困惑。
  const [collapsedGroups, setCollapsedGroups] = useState<string[]>([]);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<{
    tunnel: string;
    text: string;
  } | null>(null);
  const [loadingDiagnostics, setLoadingDiagnostics] = useState(false);

  // 版本号只信任后端返回值，避免硬编码兜底值在升级后过期。
  const appVersion = snapshot?.version ?? "—";
  const supportsPasswordAuth = snapshot?.supportsPasswordAuth ?? true;
  const platform = snapshot?.platform ?? "";

  // 密码认证依赖 Windows DPAPI，其它平台不能把"密码"作为默认推荐项。
  const defaultAuthType = (): AuthType =>
    supportsPasswordAuth ? "password" : "ssh_agent";

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

  const toggleGroupByHost = () => {
    const next = !groupByHost;
    setGroupByHost(next);
    localStorage.setItem("ssh-forward-group-by-host", String(next));
  };

  const toggleGroupCollapsed = (key: string) => {
    setCollapsedGroups((current) =>
      current.includes(key) ? current.filter((item) => item !== key) : [...current, key]
    );
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

  /**
   * 用户是否在设置面板中留下了未保存的编辑。
   *
   * 刻意只用 ref 作判据：轮询由 `setInterval` 驱动且依赖数组为空，
   * 闭包内的 state 会永远停留在初值，无法用来判断「此刻面板是否打开」。
   */
  const settingsDirty = useRef(false);

  /** 设置表单的唯一写入口：写入即视为「用户已编辑」，此后轮询不得覆盖。 */
  const updateSettingsForm = (patch: Partial<SettingsForm>) => {
    settingsDirty.current = true;
    setSettingsForm((current) => ({ ...current, ...patch }));
  };

  /**
   * 最近一次发出的 `get_snapshot` 请求序号。
   *
   * `loadData` 既被 `setInterval` 触发（不 await），也被每次操作后的 `action()` await，
   * 因此两次调用可以重叠。没有序号时，先发出的慢响应可能后到达，
   * 用较旧的数据覆盖较新的数据（例如刚创建完主机，界面却短暂回退到创建前的列表）。
   */
  const snapshotRequestId = useRef(0);

  const loadData = async () => {
    const requestId = ++snapshotRequestId.current;
    try {
      const res = await invoke<Snapshot>("get_snapshot");
      // 已有更新的请求发出，说明本次响应已过期，直接丢弃，不得写回任何状态。
      if (requestId !== snapshotRequestId.current) {
        return;
      }
      setSnapshot(res);
      // 用户正在编辑设置时不得用后端值覆盖表单，否则未保存的输入会被静默还原（B2）。
      if (res.config?.settings && !settingsDirty.current) {
        setSettingsForm(settingsFromSnapshot(res.config.settings));
      }
    } catch (error) {
      // 过期请求的失败同样不应打断界面：它对应的数据已被更新的请求取代。
      if (requestId === snapshotRequestId.current) {
        setNotice(String(error));
      }
    }
  };

  useEffect(() => {
    void loadData();

    // 窗口不可见时（最小化 / 切到其它应用）周期性刷新毫无意义：用户看不到结果，
    // 后端却仍要每 1.5 秒读一次配置、逐条探测隧道进程（每小时约 2400 次）。
    // 这里只抑制「不可见时的周期触发」，不影响操作后主动发起的 loadData。
    const timer = window.setInterval(() => {
      if (document.visibilityState === "hidden") {
        return;
      }
      void loadData();
    }, 1500);

    // 重新可见时立刻补一次，否则要等下一个 tick 才能看到最新状态。
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        void loadData();
      }
    };

    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
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

    // 基础模式下高级字段不在表单里渲染。这里必须沿用原有值而不是写 null，
    // 否则用户只是改个端口，跳板机 / ProxyCommand / 证书就会被静默清空。
    const original = hostEditing
      ? (snapshot?.config.hosts ?? []).find((item) => item.name === hostEditing)
      : undefined;

    const input = {
      name: hostForm.name,
      hostname: hostForm.hostname,
      port: Number(hostForm.port),
      username: hostForm.username,
      authType: hostForm.authType,
      password: hostForm.password || null,
      privateKey: hostForm.privateKey || null,
      jumpHostId: advancedMode
        ? hostForm.jumpHostId || null
        : original?.jump_host_id ?? null,
      proxyCommand: advancedMode
        ? hostForm.proxyCommand || null
        : original?.proxy_command ?? null,
      identitiesOnly: advancedMode
        ? hostForm.identitiesOnly
        : original?.identities_only ?? true,
      certificateFile: advancedMode
        ? hostForm.certificateFile || null
        : original?.certificate_file ?? null,
      compression: advancedMode
        ? hostForm.compression
        : original?.compression ?? false,
      customOptions: advancedMode
        ? customOptions.length
          ? customOptions
          : null
        : original?.custom_options?.length
          ? original.custom_options
          : null,
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
      setHostForm({ ...newHost(), authType: defaultAuthType() });
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
    if (
      settingsForm.hostKeyPolicy === "insecure" &&
      !window.confirm(
        "关闭主机密钥校验后，中间人攻击将无法被察觉，且服务器密钥变更也不会被拒绝。确定要使用该策略吗？"
      )
    ) {
      return;
    }
    const input = {
      hostKeyPolicy: settingsForm.hostKeyPolicy,
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

  const showDiagnostics = async (name: string) => {
    setLoadingDiagnostics(true);
    setDiagnostics({ tunnel: name, text: "" });
    try {
      const text = await invoke<string>("get_tunnel_diagnostics", { name });
      setDiagnostics({ tunnel: name, text });
    } catch (error) {
      setDiagnostics({ tunnel: name, text: String(error) });
    } finally {
      setLoadingDiagnostics(false);
    }
  };

  const hosts = snapshot?.config.hosts ?? [];
  const tunnels = snapshot?.config.tunnels ?? [];
  const statuses = snapshot?.statuses ?? {};

  /**
   * 按所属服务器把转发分组。分组顺序沿用服务器列表顺序，组内沿用配置中的原顺序。
   * `host_id` 指向不存在服务器的条目（校验层理论上已拦截，见 validation.rs）归入
   * 末尾的「未关联」分组，避免这类条目在界面上凭空消失。
   */
  const hostGroups: TunnelGroup[] = (() => {
    const hostIds = new Set(hosts.map((host) => host.id));
    const groups: TunnelGroup[] = hosts
      .map((host) => ({
        key: host.id,
        name: host.name,
        caption: `${host.username}@${host.hostname}:${host.port}`,
        tunnels: tunnels.filter((tunnel) => tunnel.host_id === host.id),
      }))
      .filter((group) => group.tunnels.length > 0);
    const orphans = tunnels.filter((tunnel) => !hostIds.has(tunnel.host_id));
    if (orphans.length) {
      groups.push({
        key: "__orphan__",
        name: "未关联服务器",
        caption: "配置中引用的服务器已不存在",
        tunnels: orphans,
      });
    }
    return groups;
  })();

  // 关闭分组时退化为单个分组且不渲染组头，等同于原有的平铺效果。
  const displayedGroups: TunnelGroup[] = groupByHost
    ? hostGroups
    : [{ key: "__flat__", name: "", caption: "", tunnels }];

  const isGroupCollapsed = (key: string) =>
    groupByHost && collapsedGroups.includes(key);

  /** 组内处于「运行中」的转发数量，用于组头统计。 */
  const runningCountOf = (group: TunnelGroup): number =>
    group.tunnels.filter(
      (tunnel) => (statuses[tunnel.name]?.state ?? "stopped") === "running"
    ).length;

  /** 生成不与现有转发重名的副本名称：`web-副本`、`web-副本2`、`web-副本3`…… */
  const duplicateTunnelName = (base: string): string => {
    const taken = new Set(tunnels.map((tunnel) => tunnel.name));
    const stem = `${base}-副本`;
    if (!taken.has(stem)) return stem;
    for (let index = 2; index <= 1000; index += 1) {
      const candidate = `${stem}${index}`;
      if (!taken.has(candidate)) return candidate;
    }
    // 极端情况（同名副本已存在上千个）退回时间戳，保证名称一定能用。
    return `${stem}${Date.now()}`;
  };

  /**
   * 以现有转发为模板打开**新建**表单（不是编辑）。
   *
   * 端口原样复制：副本的语义是「以它为模板」，自动改端口会让用户不知道新端口是多少，
   * 反而更困惑。代价是副本与原条目端口相同、无法同时启动——这一点由表单内的
   * 端口冲突提示显式告知，而不是静默处理。
   *
   * 保真：保存路径会按 `advancedMode` 丢弃它无法表达的字段（`kind` 回退为 `local`、
   * `gateway_ports` 置 `false`、`custom_options` 置 `null`）。列表并不按类型过滤，
   * 基础模式下同样会显示反向/动态隧道，因此源条目只要用到其中任何一项，
   * 就必须先把高级模式打开，否则「创建副本」会静默把反向/动态隧道变成本地转发。
   */
  const duplicateTunnel = (tunnel: Tunnel) => {
    setFormError("");
    const host = hosts.find((item) => item.id === tunnel.host_id);
    setTunnelEditing(null);

    const needsAdvancedMode =
      (tunnel.type ?? "local") !== "local" ||
      Boolean(tunnel.gateway_ports) ||
      Boolean(tunnel.custom_options?.length);
    if (needsAdvancedMode && !advancedMode) {
      setAdvancedMode(true);
      localStorage.setItem("ssh-forward-advanced-mode", "true");
    }

    setShowAdvancedTunnel(
      Boolean(tunnel.gateway_ports || tunnel.custom_options?.length)
    );
    setTunnelForm({
      name: duplicateTunnelName(tunnel.name),
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
    setPanel("tunnel");
  };

  /**
   * 正在编辑/新建的表单端口与既有条目冲突时的提示依据。
   * 端口转发在同一台机器上不能重复监听，静默冲突会让用户到"启动"时才失败且不知原因。
   */
  const portConflict = tunnels.find(
    (tunnel) =>
      tunnel.name !== tunnelEditing &&
      tunnel.local.host === tunnelForm.localHost &&
      tunnel.local.port === Number(tunnelForm.localPort)
  );

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
        : { ...newHost(), authType: defaultAuthType() }
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
    // 每次打开都从后端最新值重新开始，并清除上一次可能残留的编辑态。
    settingsDirty.current = false;
    if (snapshot?.config?.settings) {
      setSettingsForm(settingsFromSnapshot(snapshot.config.settings));
    }
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
          <button
            className="icon-btn"
            aria-label="添加服务器"
            title="添加服务器"
            onClick={() => openHost()}
          >
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
              <i aria-hidden="true"></i>
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

        {/* 设置入口常驻：主机密钥策略等安全项在基础模式下也必须可达 */}
        <button className="settings-link-btn" onClick={openSettings}>
          ⚙️ 全局网络与保活设置
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

            {/* 分组开关：多服务器场景下按服务器归类转发，避免平铺列表难以辨认归属 */}
            <button
              className={`view-toggle-btn ${groupByHost ? "active" : ""}`}
              onClick={toggleGroupByHost}
              aria-pressed={groupByHost}
              title="按服务器分组显示转发列表"
            >
              {groupByHost ? "📁 已分组" : "📁 分组"}
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
          {/* 表格视图 Table View (紧凑、自适应无横向滚动) */}
          <div className="table-wrapper">
            <table className="tunnels-table">
              <thead>
                <tr>
                  <th style={{ width: "22%" }}>名称 / 模式</th>
                  {/* 转发链路列收窄至 32%：该单元格是 flex-wrap 容器，收窄不会挤压内容。
                      操作列由 26% 放宽到 34%，容纳新增的「副本」按钮（.table-actions 为 nowrap）。 */}
                  <th style={{ width: "32%" }}>转发链路 / 路由</th>
                  <th style={{ width: "12%" }}>状态</th>
                  <th style={{ width: "34%", textAlign: "right" }}>操作</th>
                </tr>
              </thead>
              <tbody>
                {/* 用 flatMap 把「组头 + 组内条目」摊平成同级序列：tbody 只能直接放 tr，
                    而分组头本身也是一行，这样不必嵌套两层 map。 */}
                {displayedGroups.flatMap((group) => [
                  ...(groupByHost
                    ? [
                        <tr className="tunnel-group-row" key={`group-${group.key}`}>
                          <td colSpan={4}>
                            <button
                              type="button"
                              className="tunnel-group-toggle"
                              aria-expanded={!isGroupCollapsed(group.key)}
                              onClick={() => toggleGroupCollapsed(group.key)}
                            >
                              <span className="tunnel-group-caret">
                                {isGroupCollapsed(group.key) ? "▶" : "▼"}
                              </span>
                              <strong>{group.name}</strong>
                              <span className="tunnel-group-caption">{group.caption}</span>
                              <span className="tunnel-group-count">
                                {group.tunnels.length} 条转发
                                {runningCountOf(group)
                                  ? ` · ${runningCountOf(group)} 运行中`
                                  : ""}
                              </span>
                            </button>
                          </td>
                        </tr>,
                      ]
                    : []),
                  ...(isGroupCollapsed(group.key)
                    ? []
                    : group.tunnels.map((tunnel) => {
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
                                ? "SOCKS5"
                                : isRemote
                                ? "Remote"
                                : "Local"}
                            </span>
                          )}
                          {advancedMode && tunnel.gateway_ports && (
                            <span className="mode-badge gateway">共享</span>
                          )}
                        </div>
                        {status.message && (
                          <div className="table-error-hint">
                            {status.message}
                            <button
                              type="button"
                              className="error-action-btn"
                              onClick={() => void showDiagnostics(tunnel.name)}
                            >
                              📄 查看诊断日志
                            </button>
                          </div>
                        )}
                      </td>
                      <td>
                        <div className="table-route-cell">
                          <code className="port-badge">
                            {tunnel.local.host}:{tunnel.local.port}
                          </code>
                          <span className="route-arrow">➔</span>
                          {isDynamic ? (
                            <span className="table-dest-desc">
                              🌐 <code>{host?.hostname ?? "远端"}</code> (SOCKS5 全网)
                            </span>
                          ) : isRemote ? (
                            <span className="table-dest-desc">
                              📡 <code>{tunnel.remote?.host ?? "0.0.0.0"}:{tunnel.remote?.port}</code> (公网)
                            </span>
                          ) : (
                            <span className="table-dest-desc">
                              <code>{tunnel.remote?.host}:{tunnel.remote?.port}</code>
                              {host?.name ? ` (${host.name})` : ""}
                            </span>
                          )}
                        </div>
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
                              {copiedId === `tbl-socks-${tunnel.id}` ? "✓ 已复制" : "复制"}
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
                              {copiedId === `tbl-remote-${tunnel.id}` ? "✓ 已复制" : "复制"}
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
                          {/* 表格操作列是 nowrap 且已接近饱和，故用短标签「副本」；
                              可访问名统一为「创建副本」，与卡片视图保持一致。 */}
                          <button
                            className="btn-sm"
                            aria-label="创建副本"
                            title="以该条目为模板创建新条目"
                            onClick={() => duplicateTunnel(tunnel)}
                          >
                            副本
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
                      })),
                ])}
              </tbody>
            </table>
          </div>

          {/* 卡片视图 Card Grid View */}
          <div className="tunnels-grid">
            {/* 每个分组是独立的 .tunnel-group：组头独占一行，组内卡片各成一张网格，
                这样不同分组的卡片不会混排在同一行（组头若只是跨列的 grid item，
                后一组的卡片仍可能与前一组的卡片同处一行，分组在视觉上就不成立）。 */}
            {displayedGroups.map((group) => (
              <section className="tunnel-group" key={group.key}>
                {groupByHost && (
                  <button
                    type="button"
                    className="tunnel-group-toggle"
                    aria-expanded={!isGroupCollapsed(group.key)}
                    aria-controls={`tunnel-group-cards-${group.key}`}
                    onClick={() => toggleGroupCollapsed(group.key)}
                  >
                    <span className="tunnel-group-caret">
                      {isGroupCollapsed(group.key) ? "▶" : "▼"}
                    </span>
                    <strong>{group.name}</strong>
                    <span className="tunnel-group-caption">{group.caption}</span>
                    <span className="tunnel-group-count">
                      {group.tunnels.length} 条转发
                      {runningCountOf(group) ? ` · ${runningCountOf(group)} 运行中` : ""}
                    </span>
                  </button>
                )}
                <div
                  id={`tunnel-group-cards-${group.key}`}
                  className="tunnel-group-cards"
                  hidden={isGroupCollapsed(group.key)}
                >
                  {!isGroupCollapsed(group.key) &&
                    group.tunnels.map((tunnel) => {
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
                              <div className="error-actions">
                                <button
                                  type="button"
                                  className="error-action-btn"
                                  onClick={() => void showDiagnostics(tunnel.name)}
                                >
                                  📄 查看诊断日志
                                </button>
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
                              title="以该条目为模板创建新条目"
                              onClick={() => duplicateTunnel(tunnel)}
                            >
                              创建副本
                            </button>
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
              </section>
            ))}
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
                <option value="private_key">私钥文件 (.pem / id_rsa / id_ed25519)</option>
                <option value="ssh_agent">SSH Agent (系统后台密钥代理)</option>
                {(supportsPasswordAuth || hostForm.authType === "password") && (
                  <option value="password">
                    {supportsPasswordAuth
                      ? "密码 (由 Windows DPAPI 加密保存)"
                      : "密码 (当前平台不支持)"}
                  </option>
                )}
              </select>
            </label>

            {!supportsPasswordAuth && (
              <small className="field-hint">
                当前平台（{platform || "非 Windows"}）无法安全保存密码，请使用 SSH Agent 或私钥认证。
              </small>
            )}

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

            {/* 端口冲突提示：同一台机器无法重复监听同一端口。静默冲突会让用户到"启动"时
                才失败且不知原因——「创建副本」会原样复制端口，尤其容易撞上。 */}
            {portConflict && (
              <small className="field-hint field-hint-warn">
                ⚠️ 本地端口 {tunnelForm.localHost}:{tunnelForm.localPort} 已被转发「
                {portConflict.name}」占用，两者无法同时启动，请改用其它端口。
              </small>
            )}

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
                  updateSettingsForm({
                    serverAliveIntervalSeconds: Number(value),
                  })
                }
              />
              <Field
                label="探针最大未响应次数 (ServerAliveCountMax)"
                type="number"
                value={settingsForm.serverAliveCountMax}
                set={(value) =>
                  updateSettingsForm({
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
                  updateSettingsForm({
                    connectTimeoutSeconds: Number(value),
                  })
                }
              />
            </div>

            <label>
              主机密钥校验策略 (StrictHostKeyChecking)
              <select
                value={settingsForm.hostKeyPolicy}
                onChange={(e) =>
                  updateSettingsForm({
                    hostKeyPolicy: e.target.value as HostKeyPolicy,
                  })
                }
              >
                {(Object.keys(hostKeyPolicyLabels) as HostKeyPolicy[]).map((policy) => (
                  <option key={policy} value={policy}>
                    {hostKeyPolicyLabels[policy]}
                  </option>
                ))}
              </select>
            </label>
            {settingsForm.hostKeyPolicy === "insecure" ? (
              <p className="form-error" role="alert">
                当前策略不校验主机密钥：无法察觉中间人攻击，服务器更换密钥时也不会拒绝连接。
              </p>
            ) : (
              <small className="field-hint">
                可信主机记录保存在应用私有文件：
                {snapshot?.knownHostsPath ?? "（随配置文件同目录）"}
              </small>
            )}

            <div className="pair-checks" style={{ marginTop: "8px" }}>
              <label className="check">
                <input
                  type="checkbox"
                  checked={settingsForm.tcpKeepAlive}
                  onChange={(e) =>
                    updateSettingsForm({
                      tcpKeepAlive: e.target.checked,
                    })
                  }
                />
                启用系统 TCP 层保活 (TCPKeepAlive)
              </label>

              <label className="check">
                <input
                  type="checkbox"
                  checked={settingsForm.compression}
                  onChange={(e) =>
                    updateSettingsForm({
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

      {/* 诊断日志弹窗 */}
      {diagnostics && (
        <Modal
          title={`📄 诊断日志：${diagnostics.tunnel}`}
          close={() => setDiagnostics(null)}
        >
          <p className="field-hint">
            下面是 OpenSSH 连接过程中的原始输出，用于定位认证失败、Host Key 不匹配与网络不可达等问题。
          </p>
          <textarea
            className="diagnostics-output"
            readOnly
            rows={16}
            value={loadingDiagnostics ? "正在读取诊断信息..." : diagnostics.text}
          />
          <div className="form-actions">
            <button
              type="button"
              onClick={() =>
                copyToClipboard(diagnostics.text, `diag-${diagnostics.tunnel}`)
              }
              disabled={loadingDiagnostics || !diagnostics.text}
            >
              {copiedId === `diag-${diagnostics.tunnel}` ? "✓ 已复制" : "复制全部"}
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => setDiagnostics(null)}
            >
              关闭
            </button>
          </div>
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

/**
 * 对话框内可聚焦元素的选择器，供 Tab 焦点陷阱使用。
 * 对话框自身带 tabIndex={-1}，因此不会被 `[tabindex]` 分支命中。
 */
const focusableSelector = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

function Modal({
  title,
  close,
  children,
}: {
  title: string;
  close: () => void;
  children: ReactNode;
}) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const titleId = useId();

  // Esc 关闭：监听 document 而非对话框自身，这样即使焦点意外落到背景内容上也能生效。
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [close]);

  // 打开时把焦点移入对话框（聚焦容器本身而非首个可聚焦元素——本项目的对话框
  // 首个可聚焦元素是右上角「关闭」按钮，聚焦它会让读屏用户先听到"关闭"而非标题），
  // 关闭后把焦点归还给打开它的那个元素。
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    dialogRef.current?.focus();
    return () => {
      if (previous && document.contains(previous)) previous.focus();
    };
  }, []);

  // Tab 焦点陷阱：焦点在对话框内首尾循环，不逃逸到背景内容。
  const trapFocus = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Tab") return;
    const node = dialogRef.current;
    if (!node) return;
    const items = Array.from(node.querySelectorAll<HTMLElement>(focusableSelector));
    if (!items.length) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div className="modal-bg">
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        ref={dialogRef}
        tabIndex={-1}
        onKeyDown={trapFocus}
      >
        <header>
          <h2 id={titleId}>{title}</h2>
          <button type="button" aria-label="关闭" onClick={close}>
            ×
          </button>
        </header>
        {children}
      </div>
    </div>
  );
}
