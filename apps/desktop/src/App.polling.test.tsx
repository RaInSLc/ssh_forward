import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import App from "./App";

// vi.hoisted 保证 mock 句柄在 vi.mock 工厂（会被提升到 import 之前）执行时已初始化。
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));

/* ------------------------------------------------------------------ */
/* 夹具                                                                */
/* ------------------------------------------------------------------ */

const POLL_INTERVAL_MS = 1500;

/** 只需覆盖渲染所需字段：本文件关心的是「调用次数」，不是界面内容。 */
const makeSnapshot = () => ({
  path: "C:/app/config.json",
  version: "0.1.21",
  platform: "win32",
  supportsPasswordAuth: true,
  knownHostsPath: "C:/app/known_hosts",
  config: {
    settings: {
      host_key_policy: "accept_new",
      connect_timeout_seconds: 10,
      server_alive_interval_seconds: 15,
      server_alive_count_max: 3,
      tcp_keep_alive: true,
      compression: false,
    },
    hosts: [],
    tunnels: [],
  },
  statuses: {},
});

/* ------------------------------------------------------------------ */
/* 辅助                                                                */
/* ------------------------------------------------------------------ */

/** 观察到的 get_snapshot 调用次数。 */
const snapshotCalls = () =>
  invoke.mock.calls.filter((entry) => entry[0] === "get_snapshot").length;

/**
 * 覆写 `document.visibilityState` 并派发 `visibilitychange`。
 *
 * jsdom 的 `visibilityState` 是原型上的只读 getter，因此这里在实例上定义一个
 * 可配置的自有属性把它遮蔽掉；`afterEach` 里删除以还原。
 */
function setVisibility(state: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => state,
  });
  document.dispatchEvent(new Event("visibilitychange"));
}

/** 同步推进若干个轮询周期，并让由此触发的 promise 链落地。 */
async function advanceTicks(count: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS * count);
  });
}

/* ------------------------------------------------------------------ */
/* 测试                                                                */
/* ------------------------------------------------------------------ */

beforeEach(() => {
  localStorage.clear();
  invoke.mockReset();
  invoke.mockImplementation(async (command: string) =>
    command === "get_snapshot" ? makeSnapshot() : undefined,
  );
  cleanup();
  // 只接管 setInterval：RTL 的 waitFor 依赖真实 setTimeout 轮询。
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
});

afterEach(() => {
  vi.useRealTimers();
  // 删除实例上的自有属性，恢复原型上的只读 getter。
  delete (document as unknown as Record<string, unknown>).visibilityState;
});

describe("轮询感知窗口可见性（C1）", () => {
  it("窗口不可见时不再发起 get_snapshot，恢复可见时立即补一次", async () => {
    render(<App />);
    await act(async () => {});

    // 正向对照：必须先证明「可见时轮询确实在跑」，否则后面的「没增加」毫无判别力。
    const beforeVisibleTicks = snapshotCalls();
    await advanceTicks(2);
    const afterVisibleTicks = snapshotCalls();
    expect(
      afterVisibleTicks,
      "可见状态下推进 2 个周期必须产生新的 get_snapshot，否则本用例的负向断言是假通过",
    ).toBe(beforeVisibleTicks + 2);

    // 隐藏后推进 5 个周期（7.5 秒）：一次调用都不应产生。
    setVisibility("hidden");
    await advanceTicks(5);
    expect(
      snapshotCalls(),
      "窗口不可见时不应有任何后端调用",
    ).toBe(afterVisibleTicks);

    // 恢复可见：应立即补一次，而**不是**等到下一个 1.5 秒 tick。
    setVisibility("visible");
    await act(async () => {});
    expect(
      snapshotCalls(),
      "重新可见时应立即刷新一次，不等下一个 tick",
    ).toBe(afterVisibleTicks + 1);

    // 此后轮询恢复正常。
    await advanceTicks(1);
    expect(snapshotCalls()).toBe(afterVisibleTicks + 2);
  });

  it("卸载后 visibilitychange 不再触发请求（监听器已移除）", async () => {
    render(<App />);
    await act(async () => {});
    const baseline = snapshotCalls();

    cleanup();
    setVisibility("hidden");
    setVisibility("visible");
    await act(async () => {});

    expect(
      snapshotCalls(),
      "组件卸载后仍响应 visibilitychange，说明监听器泄漏",
    ).toBe(baseline);
  });
});
