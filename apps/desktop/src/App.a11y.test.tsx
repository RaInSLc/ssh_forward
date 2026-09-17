import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import App from "./App";

// vi.hoisted 保证 mock 句柄在 vi.mock 工厂（会被提升到 import 之前）执行时已初始化。
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));

const HOST_NAME = "prod-bastion";

const makeHost = () => ({
  id: "host-1",
  name: HOST_NAME,
  hostname: "10.0.0.5",
  port: 22,
  username: "deploy",
  auth: { type: "ssh_agent" },
  identities_only: true,
  enabled: true,
});

const makeSnapshot = () => ({
  path: "C:/Users/Administrator/AppData/Roaming/ssh-forward/config.json",
  version: "0.1.17",
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
    hosts: [makeHost()],
    tunnels: [],
  },
  statuses: {},
});

/**
 * 侧栏主机列表容器。分组开启后服务器名也会出现在**分组头**里（卡片视图与表格视图各一份），
 * 因此按文本或 role 查询主机行时必须限定到侧栏，否则会命中分组头。
 */
function hostList(): HTMLElement {
  const node = document.querySelector(".host-list");
  if (!node) throw new Error("未找到侧栏主机列表 .host-list");
  return node as HTMLElement;
}

async function renderApp() {
  invoke.mockImplementation(async (command: string) =>
    command === "get_snapshot" ? makeSnapshot() : undefined,
  );
  render(<App />);
  await within(hostList()).findByText(HOST_NAME);
}

function hostRow(): HTMLElement {
  return within(hostList()).getByRole("button", { name: new RegExp(HOST_NAME) });
}

/** 打开服务器编辑对话框（App 中 5 个弹窗共用同一个 Modal 组件）。 */
async function openHostDialog(): Promise<HTMLElement> {
  fireEvent.click(hostRow());
  return screen.findByRole("dialog");
}

beforeEach(() => {
  localStorage.clear();
  invoke.mockReset();
  cleanup();
});

describe("对话框语义与键盘操作", () => {
  it("弹窗暴露 dialog 语义、aria-modal，并以标题作为可访问名", async () => {
    await renderApp();
    const dialog = await openHostDialog();

    expect(dialog.getAttribute("aria-modal")).toBe("true");

    // aria-labelledby 必须指向标题元素，且该元素文本即对话框名。
    const labelledBy = dialog.getAttribute("aria-labelledby");
    expect(labelledBy).toBeTruthy();
    const heading = dialog.querySelector("h2");
    expect(heading?.id).toBe(labelledBy);
    expect(heading?.textContent).toBe("编辑服务器");

    // 按可访问名也能查到同一个元素。
    expect(screen.getByRole("dialog", { name: "编辑服务器" })).toBe(dialog);
  });

  it("打开弹窗时把焦点移入对话框", async () => {
    await renderApp();
    const dialog = await openHostDialog();

    await waitFor(() => {
      expect(document.activeElement).toBe(dialog);
    });
  });

  it("按 Esc 关闭弹窗", async () => {
    await renderApp();
    await openHostDialog();

    fireEvent.keyDown(document, { key: "Escape" });

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBe(null);
    });
  });

  it("Tab 焦点陷阱：在对话框内首尾循环，不逃逸到背景内容", async () => {
    await renderApp();
    const dialog = await openHostDialog();

    const first = within(dialog).getByRole("button", { name: "关闭" });
    const last = within(dialog).getByRole("button", { name: "保存服务器" });

    // 在最后一个可聚焦元素上按 Tab → 回到第一个
    last.focus();
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(document.activeElement).toBe(first);

    // 在第一个可聚焦元素上按 Shift+Tab → 跳到最后一个
    fireEvent.keyDown(dialog, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(last);

    // 中间位置的 Tab 不拦截（由浏览器自行处理）
    const middle = within(dialog).getByLabelText("SSH 用户名");
    middle.focus();
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(document.activeElement).toBe(middle);
  });

  it("关闭弹窗后焦点归还给触发元素", async () => {
    await renderApp();
    const row = hostRow();
    row.focus();

    const dialog = await openHostDialog();
    await waitFor(() => expect(document.activeElement).toBe(dialog));

    fireEvent.keyDown(document, { key: "Escape" });

    await waitFor(() => expect(screen.queryByRole("dialog")).toBe(null));
    expect(document.activeElement).toBe(row);
  });
});

describe("控件的可访问名与装饰元素", () => {
  it("弹窗关闭按钮是纯符号，必须有可访问名", async () => {
    await renderApp();
    const dialog = await openHostDialog();

    const close = within(dialog).getByRole("button", { name: "关闭" });
    expect(close.textContent).toBe("×");
  });

  it("侧栏纯符号「+」按钮必须有可访问名", async () => {
    await renderApp();

    // 精确匹配 "添加服务器"：另一个按钮的文本是 "+ 添加服务器"，不会命中。
    const addButton = screen.getByRole("button", { name: "添加服务器" });
    expect(addButton.textContent?.trim()).toBe("+");
  });

  it("主机行的状态点是无语义装饰，必须对辅助技术隐藏", async () => {
    await renderApp();

    const dot = document.querySelector(".host-row i");
    expect(dot).toBeTruthy();
    expect(dot?.getAttribute("aria-hidden")).toBe("true");
    // 装饰元素不应有文本内容，避免被读屏念出空节点
    expect(dot?.textContent).toBe("");
  });
});
