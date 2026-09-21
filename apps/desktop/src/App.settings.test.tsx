import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import App from "./App";

// vi.hoisted 保证 mock 句柄在 vi.mock 工厂（会被提升到 import 之前）执行时已初始化。
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));

/* ------------------------------------------------------------------ */
/* 夹具                                                                */
/* ------------------------------------------------------------------ */

const HOST_NAME = "prod-bastion";

const makeHost = () => ({
  id: "host-1",
  name: HOST_NAME,
  hostname: "10.0.0.5",
  port: 22,
  username: "deploy",
  auth: { type: "ssh_agent" },
  enabled: true,
});

/** 快照夹具：只需覆盖设置面板用到的字段。 */
const makeSnapshot = (serverAliveIntervalSeconds: number, hostName = HOST_NAME) => ({
  path: "C:/app/config.json",
  version: "0.1.20",
  platform: "win32",
  supportsPasswordAuth: true,
  knownHostsPath: "C:/app/known_hosts",
  config: {
    settings: {
      host_key_policy: "accept_new",
      connect_timeout_seconds: 10,
      server_alive_interval_seconds: serverAliveIntervalSeconds,
      server_alive_count_max: 3,
      tcp_keep_alive: true,
      compression: false,
    },
    hosts: [{ ...makeHost(), name: hostName }],
    tunnels: [],
  },
  statuses: {},
});

/** 可手动决定 settle 时机的 promise，用于制造乱序响应。 */
function createDeferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((settle, fail) => {
    resolve = settle;
    reject = fail;
  });
  return { promise, resolve, reject };
}

const INTERVAL_LABEL = "心跳探针周期 (ServerAliveInterval，秒)";

/* ------------------------------------------------------------------ */
/* 辅助                                                                */
/* ------------------------------------------------------------------ */

/** 侧栏主机列表容器：主机名也会出现在分组头里，按文本查询必须限定到侧栏。 */
function hostList(): HTMLElement {
  const node = document.querySelector(".host-list");
  if (!node) throw new Error("未找到侧栏主机列表 .host-list");
  return node as HTMLElement;
}

/** 让 `get_snapshot` 返回指定心跳周期的快照。 */
function serveSnapshot(serverAliveIntervalSeconds: number) {
  invoke.mockImplementation(async (command: string) =>
    command === "get_snapshot" ? makeSnapshot(serverAliveIntervalSeconds) : undefined,
  );
}

/** 渲染 App 并等待首次 get_snapshot 落地。 */
async function renderApp(serverAliveIntervalSeconds: number) {
  serveSnapshot(serverAliveIntervalSeconds);
  render(<App />);
  await within(hostList()).findByText(HOST_NAME);
}

const openSettingsPanel = () => {
  fireEvent.click(screen.getByRole("button", { name: /全局网络与保活设置/ }));
};

const intervalInput = () => screen.getByLabelText(INTERVAL_LABEL) as HTMLInputElement;

/** 推进轮询定时器，并让由此触发的 promise 链落地。 */
async function advancePolling(milliseconds: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(milliseconds);
  });
}

/**
 * 同步推进一个轮询周期。
 *
 * 与 `advancePolling` 的区别：这里用同步的 `advanceTimersByTime`，因此**不会等待**
 * 由该次轮询发出、尚未 resolve 的 `get_snapshot`——正是制造「两次请求重叠」所需。
 */
async function firePollingTick() {
  await act(async () => {
    vi.advanceTimersByTime(POLL_INTERVAL_MS);
  });
}

/** 观察到的 get_snapshot 调用次数：用于证明轮询确实执行过。 */
const snapshotCalls = () =>
  invoke.mock.calls.filter((entry) => entry[0] === "get_snapshot").length;

const POLL_INTERVAL_MS = 1500;

/* ------------------------------------------------------------------ */
/* 测试                                                                */
/* ------------------------------------------------------------------ */

beforeEach(() => {
  localStorage.clear();
  invoke.mockReset();
  cleanup();
  // 只接管 setInterval：RTL 的 waitFor 依赖真实 setTimeout 轮询
  // （它只识别 Jest 的假定时器，不认识 Vitest 的），接管 setTimeout 会让 waitFor 永久挂起。
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("设置面板与 1.5 秒轮询的交互（B2 回归）", () => {
  it("用户在面板中的未保存输入不会被轮询还原", async () => {
    await renderApp(15);
    openSettingsPanel();
    expect(intervalInput().value).toBe("15");

    fireEvent.change(intervalInput(), { target: { value: "99" } });
    expect(intervalInput().value).toBe("99");

    // 推进三个轮询周期：修复前这里会把输入静默还原为后端的 15。
    const before = snapshotCalls();
    await advancePolling(POLL_INTERVAL_MS * 3);

    // 先证明轮询真的执行过，否则「值没变」可能只是定时器压根没触发。
    expect(snapshotCalls()).toBeGreaterThan(before);
    expect(intervalInput().value).toBe("99");
  });

  it("未编辑时轮询仍会同步后端的新值", async () => {
    await renderApp(15);
    openSettingsPanel();
    expect(intervalInput().value).toBe("15");

    // 后端在下一个周期返回了不同的值：表单未处于编辑态，应当跟随。
    serveSnapshot(30);
    await advancePolling(POLL_INTERVAL_MS + 100);

    expect(intervalInput().value).toBe("30");
  });

  it("放弃编辑后重新打开面板会回到后端值", async () => {
    await renderApp(15);
    openSettingsPanel();
    fireEvent.change(intervalInput(), { target: { value: "99" } });
    expect(intervalInput().value).toBe("99");

    // Esc 关闭 = 放弃未保存的编辑（Modal 支持 Esc，见 App.tsx 的 Modal 实现）。
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByLabelText(INTERVAL_LABEL)).toBeNull();

    openSettingsPanel();
    expect(intervalInput().value).toBe("15");
  });
});

describe("轮询乱序响应（B3 回归）", () => {
  it("先发出的慢响应不得覆盖后发出的新数据", async () => {
    await renderApp(15);
    expect(within(hostList()).getByText(HOST_NAME)).toBeTruthy();

    // 之后两次轮询各返回一个可手动决定 resolve 时机的响应。
    const slow = createDeferred<ReturnType<typeof makeSnapshot>>();
    const fast = createDeferred<ReturnType<typeof makeSnapshot>>();
    const queue = [slow, fast];
    invoke.mockImplementation(async (command: string) => {
      if (command !== "get_snapshot") return undefined;
      const next = queue.shift();
      return next ? next.promise : makeSnapshot(15);
    });

    // 发出请求 #1（慢响应，此刻仍未 resolve）。
    await firePollingTick();
    // 发出请求 #2（与 #1 重叠）。
    await firePollingTick();

    // 后发出的 #2 先返回新数据。
    await act(async () => {
      fast.resolve(makeSnapshot(15, "newer-host"));
    });
    expect(within(hostList()).getByText("newer-host")).toBeTruthy();

    // 先发出的 #1 后返回旧数据——修复前它会覆盖掉上面刚落地的新数据。
    await act(async () => {
      slow.resolve(makeSnapshot(15, "stale-host"));
    });

    expect(within(hostList()).queryByText("stale-host")).toBeNull();
    expect(within(hostList()).getByText("newer-host")).toBeTruthy();
  });

  it("过期请求的失败不会打断界面", async () => {
    await renderApp(15);

    const slow = createDeferred<ReturnType<typeof makeSnapshot>>();
    const fast = createDeferred<ReturnType<typeof makeSnapshot>>();
    const queue = [slow, fast];
    invoke.mockImplementation(async (command: string) => {
      if (command !== "get_snapshot") return undefined;
      const next = queue.shift();
      return next ? next.promise : makeSnapshot(15);
    });

    await firePollingTick();
    await firePollingTick();

    await act(async () => {
      fast.resolve(makeSnapshot(15, "newer-host"));
    });
    // 已被取代的请求随后失败：不应把错误提示推给用户。
    await act(async () => {
      slow.reject(new Error("连接已中断"));
    });
    // 拒绝的传播链比 resolve 多一跳，需要再 flush 一轮才能让 setState 落地。
    await act(async () => {});

    expect(within(hostList()).getByText("newer-host")).toBeTruthy();
    expect(screen.queryByText(/连接已中断/)).toBeNull();
  });
});
