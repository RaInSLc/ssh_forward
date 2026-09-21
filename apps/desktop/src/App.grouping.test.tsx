import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import App from "./App";

// vi.hoisted 保证 mock 句柄在 vi.mock 工厂（会被提升到 import 之前）执行时已初始化。
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));

/* ------------------------------------------------------------------ */
/* 夹具                                                                */
/* ------------------------------------------------------------------ */

const makeHost = (id: string, name: string, hostname: string) => ({
  id,
  name,
  hostname,
  port: 22,
  username: "deploy",
  auth: { type: "ssh_agent" },
  identities_only: true,
  enabled: true,
});

const HOST_A = makeHost("host-a", "prod-a", "10.0.0.1");
const HOST_B = makeHost("host-b", "prod-b", "10.0.0.2");

const makeTunnel = (
  id: string,
  name: string,
  hostId: string,
  localPort: number,
  extra: Record<string, unknown> = {},
) => ({
  id,
  name,
  host_id: hostId,
  type: "local",
  local: { host: "127.0.0.1", port: localPort },
  remote: { host: "127.0.0.1", port: 80 },
  auto_open_browser: false,
  enabled: true,
  ...extra,
});

const makeSnapshot = (hosts: unknown[], tunnels: unknown[]) => ({
  path: "C:/Users/Administrator/AppData/Roaming/ssh-forward/config.json",
  version: "0.1.18",
  platform: "win32",
  supportsPasswordAuth: true,
  config: {
    settings: {
      host_key_policy: "accept_new",
      connect_timeout_seconds: 10,
      server_alive_interval_seconds: 15,
      server_alive_count_max: 3,
      tcp_keep_alive: true,
      compression: false,
    },
    hosts,
    tunnels,
  },
  statuses: {},
});

/* ------------------------------------------------------------------ */
/* 辅助                                                                */
/* ------------------------------------------------------------------ */

/**
 * 侧栏主机列表容器。分组开启后服务器名也会出现在**分组头**里（卡片与表格视图各一份），
 * 因此按文本或 role 查询主机行时必须限定到侧栏。
 */
function hostList(): HTMLElement {
  const node = document.querySelector(".host-list");
  if (!node) throw new Error("未找到侧栏主机列表 .host-list");
  return node as HTMLElement;
}

/** 卡片视图容器（表格视图与卡片视图同时存在于 DOM，仅靠 CSS 切换显隐）。 */
function cardView(): HTMLElement {
  const node = document.querySelector(".tunnels-grid");
  if (!node) throw new Error("未找到卡片视图容器 .tunnels-grid");
  return node as HTMLElement;
}

async function renderApp(hosts: unknown[], tunnels: unknown[]) {
  invoke.mockImplementation(async (command: string) =>
    command === "get_snapshot" ? makeSnapshot(hosts, tunnels) : undefined,
  );
  render(<App />);
  await within(hostList()).findByText((hosts[0] as { name: string }).name);
}

/** 卡片视图里的分组头按钮。 */
function groupToggle(name: string): HTMLElement {
  return within(cardView()).getByRole("button", { name: new RegExp(name) });
}

function groupCount(): number {
  return cardView().querySelectorAll(".tunnel-group").length;
}

function groupHeaders(): HTMLElement[] {
  return Array.from(cardView().querySelectorAll<HTMLElement>(".tunnel-group-toggle"));
}

/** 按隧道名定位卡片（卡片标题是 <h2>）。 */
function tunnelCard(name: string): HTMLElement {
  const heading = within(cardView()).getByRole("heading", { name, level: 2 });
  const card = heading.closest("article");
  if (!card) throw new Error(`未找到转发卡片「${name}」`);
  return card as HTMLElement;
}

function payloadOf(command: string): any {
  const calls = invoke.mock.calls.filter((entry) => entry[0] === command);
  if (!calls.length) {
    throw new Error(
      `未观察到对 ${command} 的调用，实际调用：${JSON.stringify(
        invoke.mock.calls.map((entry) => entry[0]),
      )}`,
    );
  }
  return calls[calls.length - 1][1];
}

beforeEach(() => {
  localStorage.clear();
  invoke.mockReset();
  cleanup();
});

/* ------------------------------------------------------------------ */
/* 分组                                                                */
/* ------------------------------------------------------------------ */

describe("按服务器分组", () => {
  const hosts = [HOST_A, HOST_B];
  const tunnels = [
    makeTunnel("t1", "web-a", "host-a", 18001),
    makeTunnel("t2", "db-a", "host-a", 18002),
    makeTunnel("t3", "web-b", "host-b", 18003),
  ];

  it("卡片视图按服务器分组，组头显示服务器信息与转发数量", async () => {
    await renderApp(hosts, tunnels);

    expect(groupCount()).toBe(2);
    expect(groupHeaders()).toHaveLength(2);

    // 分组顺序沿用服务器列表顺序，组内沿用配置顺序。
    const [first, second] = groupHeaders();
    expect(first.textContent).toContain("prod-a");
    expect(first.textContent).toContain("deploy@10.0.0.1:22");
    expect(first.textContent).toContain("2 条转发");
    expect(second.textContent).toContain("prod-b");
    expect(second.textContent).toContain("1 条转发");
  });

  it("host_id 指向不存在服务器的转发归入「未关联服务器」分组", async () => {
    await renderApp(hosts, [...tunnels, makeTunnel("t9", "orphan", "host-missing", 18009)]);

    expect(groupCount()).toBe(3);
    const last = groupHeaders()[2];
    expect(last.textContent).toContain("未关联服务器");
    expect(last.textContent).toContain("1 条转发");
    // 关键：这类条目不能凭空消失。
    expect(within(cardView()).getByRole("heading", { name: "orphan", level: 2 })).toBeTruthy();
  });

  it("点击组头可折叠与展开该分组，aria-expanded 同步", async () => {
    await renderApp(hosts, tunnels);

    const toggle = groupToggle("prod-a");
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(within(cardView()).queryByRole("heading", { name: "web-a", level: 2 })).toBeTruthy();

    fireEvent.click(toggle);

    expect(groupToggle("prod-a").getAttribute("aria-expanded")).toBe("false");
    const collapsedPanel = cardView().querySelectorAll<HTMLElement>(".tunnel-group-cards")[0];
    expect(collapsedPanel.hidden).toBe(true);
    expect(groupToggle("prod-a").getAttribute("aria-controls")).toBe(collapsedPanel.id);
    expect(within(cardView()).queryByRole("heading", { name: "web-a", level: 2 })).toBe(null);
    expect(within(cardView()).queryByRole("heading", { name: "db-a", level: 2 })).toBe(null);
    // 只折叠本组：另一组不受影响。
    expect(within(cardView()).getByRole("heading", { name: "web-b", level: 2 })).toBeTruthy();

    fireEvent.click(groupToggle("prod-a"));
    expect(groupToggle("prod-a").getAttribute("aria-expanded")).toBe("true");
    expect(within(cardView()).getByRole("heading", { name: "web-a", level: 2 })).toBeTruthy();
  });

  it("分组默认开启；关闭后持久化并退化为平铺（无组头）", async () => {
    await renderApp(hosts, tunnels);

    // 默认开启：未写入 localStorage。
    expect(localStorage.getItem("ssh-forward-group-by-host")).toBe(null);
    expect(groupHeaders()).toHaveLength(2);

    fireEvent.click(screen.getByRole("button", { name: /已分组/ }));

    expect(localStorage.getItem("ssh-forward-group-by-host")).toBe("false");
    expect(groupHeaders()).toHaveLength(0);
    // 关闭分组后所有转发仍然可见。
    expect(cardView().querySelectorAll("article.tunnel-card")).toHaveLength(3);

    // 重新挂载后仍保持关闭。
    cleanup();
    await renderApp(hosts, tunnels);
    expect(groupHeaders()).toHaveLength(0);
    expect(cardView().querySelectorAll("article.tunnel-card")).toHaveLength(3);
  });
});

/* ------------------------------------------------------------------ */
/* 创建副本                                                            */
/* ------------------------------------------------------------------ */

describe("创建副本", () => {
  it("以「新建」打开表单，且名称自动去重", async () => {
    await renderApp(
      [HOST_A],
      [
        makeTunnel("t1", "web-a", "host-a", 18001),
        makeTunnel("t2", "web-a-副本", "host-a", 18002),
      ],
    );

    fireEvent.click(within(tunnelCard("web-a")).getByRole("button", { name: "创建副本" }));

    // 必须是新建而非编辑，否则保存会覆盖原条目。
    expect(await screen.findByRole("dialog", { name: "新建 Tunnel" })).toBeTruthy();

    const nameInput = screen.getByLabelText("名称 (用于标记区分)") as HTMLInputElement;
    // web-a-副本 已被占用，因此应生成 web-a-副本2。
    expect(nameInput.value).toBe("web-a-副本2");
  });

  it("完整保留类型、端点、GatewayPorts、自定义 -o 与自动打开浏览器", async () => {
    localStorage.setItem("ssh-forward-advanced-mode", "true");
    await renderApp(
      [HOST_A],
      [
        makeTunnel("t1", "expose-web", "host-a", 3000, {
          type: "remote",
          remote: { host: "0.0.0.0", port: 8080 },
          gateway_ports: true,
          custom_options: ["ExitOnForwardFailure=yes"],
          auto_open_browser: true,
        }),
      ],
    );

    fireEvent.click(within(tunnelCard("expose-web")).getByRole("button", { name: "创建副本" }));
    await screen.findByRole("dialog", { name: "新建 Tunnel" });

    // 表单预填：远端端点取自原条目（remote 模式下 remote 是监听端，故标签不同）。
    const remoteHost = screen.getByLabelText(
      "远程绑定地址 (通常 0.0.0.0 或 127.0.0.1)"
    ) as HTMLInputElement;
    const remotePort = screen.getByLabelText(
      "远程公网端口 (外部访问此端口)"
    ) as HTMLInputElement;
    expect(remoteHost.value).toBe("0.0.0.0");
    expect(remotePort.value).toBe("8080");
    expect((screen.getByLabelText("本地服务端口") as HTMLInputElement).value).toBe("3000");

    fireEvent.click(screen.getByRole("button", { name: "保存 Tunnel" }));

    await waitFor(() => {
      expect(invoke.mock.calls.some((entry) => entry[0] === "create_tunnel")).toBe(true);
    });

    const payload = payloadOf("create_tunnel");
    expect(payload.input.name).toBe("expose-web-副本");
    expect(payload.input.kind).toBe("remote");
    expect(payload.input.remoteHost).toBe("0.0.0.0");
    expect(payload.input.remotePort).toBe(8080);
    expect(payload.input.localPort).toBe(3000);
    expect(payload.input.gatewayPorts).toBe(true);
    expect(payload.input.customOptions).toEqual(["ExitOnForwardFailure=yes"]);
    expect(payload.input.autoOpenBrowser).toBe(true);
  });

  it("保存副本走 create_tunnel，不会覆盖原条目", async () => {
    await renderApp([HOST_A], [makeTunnel("t1", "web-a", "host-a", 18001)]);

    fireEvent.click(within(tunnelCard("web-a")).getByRole("button", { name: "创建副本" }));
    await screen.findByRole("dialog", { name: "新建 Tunnel" });
    fireEvent.click(screen.getByRole("button", { name: "保存 Tunnel" }));

    await waitFor(() => {
      expect(invoke.mock.calls.some((entry) => entry[0] === "create_tunnel")).toBe(true);
    });

    expect(invoke.mock.calls.some((entry) => entry[0] === "edit_tunnel")).toBe(false);
    // 新建请求不带 originalName。
    expect(payloadOf("create_tunnel").originalName).toBeUndefined();
  });

  it("副本端口与原条目相同时给出显式警告（否则要到启动才失败）", async () => {
    await renderApp([HOST_A], [makeTunnel("t1", "web-a", "host-a", 18001)]);

    fireEvent.click(within(tunnelCard("web-a")).getByRole("button", { name: "创建副本" }));
    await screen.findByRole("dialog", { name: "新建 Tunnel" });

    // 副本原样复制端口，因此必然与原条目冲突——必须显式告知，而不是静默放行。
    const warning = screen.getByText(/已被转发「web-a」占用/);
    expect(warning.textContent).toContain("18001");
  });

  /**
   * 基础模式（默认）下保存路径会把 kind 回退为 local、gateway_ports 置 false、
   * custom_options 置 null。而列表并不按类型过滤，反向/动态隧道在基础模式下同样可见，
   * 因此这两条用例守护的是「副本不得被静默降级」。
   */
  it("基础模式下复制反向隧道会自动打开高级模式，kind 不被降级为 local", async () => {
    await renderApp(
      [HOST_A],
      [
        makeTunnel("t1", "expose-web", "host-a", 3000, {
          type: "remote",
          remote: { host: "0.0.0.0", port: 8080 },
        }),
      ],
    );

    // 前置条件：确为默认基础模式。
    expect(localStorage.getItem("ssh-forward-advanced-mode")).toBe(null);
    expect(screen.getByRole("button", { name: /开启高级设置/ })).toBeTruthy();

    fireEvent.click(within(tunnelCard("expose-web")).getByRole("button", { name: "创建副本" }));
    await screen.findByRole("dialog", { name: "新建 Tunnel" });

    // 高级模式被自动打开并持久化。
    expect(localStorage.getItem("ssh-forward-advanced-mode")).toBe("true");
    expect(screen.getByRole("button", { name: /高级模式：已开启/ })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "保存 Tunnel" }));
    await waitFor(() => {
      expect(invoke.mock.calls.some((entry) => entry[0] === "create_tunnel")).toBe(true);
    });

    expect(payloadOf("create_tunnel").input.kind).toBe("remote");
    expect(payloadOf("create_tunnel").input.remotePort).toBe(8080);
  });

  it("基础模式下复制带 GatewayPorts 的本地隧道同样打开高级模式，字段不被丢弃", async () => {
    await renderApp(
      [HOST_A],
      [
        makeTunnel("t1", "shared-web", "host-a", 3000, {
          gateway_ports: true,
          custom_options: ["ExitOnForwardFailure=yes"],
        }),
      ],
    );

    expect(localStorage.getItem("ssh-forward-advanced-mode")).toBe(null);

    fireEvent.click(within(tunnelCard("shared-web")).getByRole("button", { name: "创建副本" }));
    await screen.findByRole("dialog", { name: "新建 Tunnel" });

    expect(localStorage.getItem("ssh-forward-advanced-mode")).toBe("true");

    fireEvent.click(screen.getByRole("button", { name: "保存 Tunnel" }));
    await waitFor(() => {
      expect(invoke.mock.calls.some((entry) => entry[0] === "create_tunnel")).toBe(true);
    });

    const payload = payloadOf("create_tunnel");
    expect(payload.input.kind).toBe("local");
    expect(payload.input.gatewayPorts).toBe(true);
    expect(payload.input.customOptions).toEqual(["ExitOnForwardFailure=yes"]);
  });
});
