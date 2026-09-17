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

const HOST_NAME = "prod-bastion";

/** 主机夹具：高级字段全部为非默认值，这样「被清空」与「被保留」可明确区分。 */
const makeHost = () => ({
  id: "host-1",
  name: HOST_NAME,
  hostname: "10.0.0.5",
  port: 22,
  username: "deploy",
  auth: { type: "ssh_agent" },
  jump_host_id: "host-jump",
  proxy_command: "nc %h %p",
  identities_only: false,
  certificate_file: "/home/deploy/.ssh/id_ed25519-cert.pub",
  compression: true,
  custom_options: ["ServerAliveCountMax=9"],
  enabled: true,
});

/** 反向转发隧道：基础模式下其 type 会被强制改写为 local（见文件末尾的现状记录测试）。 */
const makeRemoteTunnel = () => ({
  id: "tunnel-remote",
  name: "expose-web",
  host_id: "host-1",
  type: "remote",
  local: { host: "127.0.0.1", port: 3000 },
  remote: { host: "0.0.0.0", port: 8080 },
  gateway_ports: true,
  custom_options: ["ExitOnForwardFailure=yes"],
  auto_open_browser: false,
  enabled: true,
});

const makeSnapshot = (tunnels: unknown[] = []) => ({
  path: "C:/Users/Administrator/AppData/Roaming/ssh-forward/config.json",
  version: "0.1.17",
  platform: "win32",
  supportsPasswordAuth: true,
  knownHostsPath: "C:/Users/Administrator/.ssh/ssh_forward_known_hosts",
  config: {
    settings: {
      host_key_policy: "accept_new",
      connect_timeout_seconds: 10,
      server_alive_interval_seconds: 15,
      server_alive_count_max: 3,
      tcp_keep_alive: true,
      compression: false,
    },
    hosts: [makeHost()],
    tunnels,
  },
  statuses: {},
});

/* ------------------------------------------------------------------ */
/* 辅助                                                                */
/* ------------------------------------------------------------------ */

/** 取某条命令最后一次调用的 payload。 */
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

/**
 * 侧栏主机列表容器。分组开启后服务器名也会出现在**分组头**里（卡片视图与表格视图各一份），
 * 因此按文本或 role 查询主机行时必须限定到侧栏，否则会命中分组头。
 */
function hostList(): HTMLElement {
  const node = document.querySelector(".host-list");
  if (!node) throw new Error("未找到侧栏主机列表 .host-list");
  return node as HTMLElement;
}

/** 渲染 App 并等待首次 get_snapshot 完成。 */
async function renderApp(tunnels: unknown[] = []) {
  invoke.mockImplementation(async (command: string) =>
    command === "get_snapshot" ? makeSnapshot(tunnels) : undefined,
  );
  render(<App />);
  await within(hostList()).findByText(HOST_NAME);
}

/** 点击表单里的提交按钮（<button> 未写 type，默认为 submit）。 */
function clickSubmit(label: string) {
  fireEvent.click(screen.getByRole("button", { name: label }));
}

/**
 * 隧道列表**同时**渲染卡片视图（`.tunnels-grid`）与表格视图（`.table-wrapper`），
 * 仅靠 `.view-grid` / `.view-table` 这两个 CSS 类切换显隐（styles.css:494-505），
 * 因此两套按钮都在 DOM 里，按 role 查询必须限定到具体视图容器。
 */
function tunnelCardButton(label: string): HTMLElement {
  const container = document.querySelector(".tunnels-grid");
  if (!container) throw new Error("未找到卡片视图容器 .tunnels-grid");
  return within(container as HTMLElement).getByRole("button", { name: label });
}

/* ------------------------------------------------------------------ */
/* 测试                                                                */
/* ------------------------------------------------------------------ */

beforeEach(() => {
  // 基础模式 = localStorage 中不存 "ssh-forward-advanced-mode" 的 "true"。
  localStorage.clear();
  invoke.mockReset();
  cleanup();
});

describe("基础模式下的保存语义", () => {
  it("编辑主机时保留未渲染的高级字段（A1 回归）", async () => {
    await renderApp();

    // 进入基础模式：断言高级设置开关确实未开启。
    expect(screen.getByRole("button", { name: /开启高级设置/ })).toBeTruthy();

    fireEvent.click(within(hostList()).getByRole("button", { name: new RegExp(HOST_NAME) }));
    const portInput = (await screen.findByLabelText("SSH 端口")) as HTMLInputElement;
    expect(portInput.value).toBe("22");

    // 用户只改端口，不碰任何高级字段。
    fireEvent.change(portInput, { target: { value: "2222" } });
    clickSubmit("保存服务器");

    await waitFor(() => {
      expect(invoke.mock.calls.some((entry) => entry[0] === "edit_host")).toBe(true);
    });

    const payload = payloadOf("edit_host");
    expect(payload.originalName).toBe(HOST_NAME);
    expect(payload.input.port).toBe(2222);

    // 核心断言：未在表单中渲染的字段必须沿用原值，而不是被写成 null / false。
    expect(payload.input.jumpHostId).toBe("host-jump");
    expect(payload.input.proxyCommand).toBe("nc %h %p");
    expect(payload.input.certificateFile).toBe("/home/deploy/.ssh/id_ed25519-cert.pub");
    expect(payload.input.compression).toBe(true);
    expect(payload.input.customOptions).toEqual(["ServerAliveCountMax=9"]);
    // identities_only 原值为 false，若退化为默认值 true 即说明原值未被沿用。
    expect(payload.input.identitiesOnly).toBe(false);
  });

  it("高级模式下高级字段取自表单而非原值（契约测试）", async () => {
    localStorage.setItem("ssh-forward-advanced-mode", "true");
    await renderApp();

    fireEvent.click(within(hostList()).getByRole("button", { name: new RegExp(HOST_NAME) }));
    const portInput = (await screen.findByLabelText("SSH 端口")) as HTMLInputElement;
    fireEvent.change(portInput, { target: { value: "2222" } });
    clickSubmit("保存服务器");

    await waitFor(() => {
      expect(invoke.mock.calls.some((entry) => entry[0] === "edit_host")).toBe(true);
    });

    // 高级模式下表单由 openHost 用原值预填，因此这些值仍应等于原值——
    // 但若表单被清空，写出的就应是 null。这里同时锁住「预填」与「提交」两端。
    const payload = payloadOf("edit_host");
    expect(payload.input.jumpHostId).toBe("host-jump");
    expect(payload.input.proxyCommand).toBe("nc %h %p");
    expect(payload.input.compression).toBe(true);
    expect(payload.input.customOptions).toEqual(["ServerAliveCountMax=9"]);
  });
});

describe("隧道保存语义", () => {
  it("现状记录：基础模式下编辑反向转发隧道会被改写为本地转发", async () => {
    // 本用例记录的是**当前实现的行为**，不是期望行为。
    // `saveTunnel` 在基础模式下把 kind 硬编码为 "local"（App.tsx:443），
    // 而隧道卡片的「编辑」按钮未受 advancedMode 守卫（App.tsx:995），
    // 因此基础模式下编辑一条 -R 隧道会静默把它变成 -L。
    // 该行为源自 v0.1.15 的「基础/高级模式分级」设计，不属于 P0 的 A1 修复范围。
    // 一旦该问题被修复，本用例应当失败——届时应改写为断言原值被保留。
    await renderApp([makeRemoteTunnel()]);

    fireEvent.click(tunnelCardButton("编辑"));
    await screen.findByText("编辑 Tunnel");
    clickSubmit("保存 Tunnel");

    await waitFor(() => {
      expect(invoke.mock.calls.some((entry) => entry[0] === "edit_tunnel")).toBe(true);
    });

    const payload = payloadOf("edit_tunnel");
    expect(payload.originalName).toBe("expose-web");
    expect(payload.input.kind).toBe("local");
    expect(payload.input.gatewayPorts).toBe(false);
    expect(payload.input.customOptions).toBe(null);
  });
});
