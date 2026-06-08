import { describe, expect, it } from "vitest";

import { parseSnapshot } from "../src/lib/ssh/hostSnapshot";

// The snapshot is one marker-delimited stdout. These tests focus on the env
// block (Python / conda / virtualenv / `module`), which runs under a login
// shell with internal `@@` markers, and on its graceful degradation.
describe("parseSnapshot — environment block", () => {
  const withEnv = (envBody: string) => `###USER\nresearcher\n###ENV\n${envBody}`;

  it("parses python, conda env, and loaded modules", () => {
    const snap = parseSnapshot(
      withEnv(
        [
          "@@PY Python 3.11.7",
          "@@CONDA ml-train",
          "@@VENV ",
          "@@MODULE",
          "Currently Loaded Modules:",
          "  1) gcc/11.3.0   2) cuda/12.4   3) cudnn/8.9.2",
        ].join("\n"),
      ),
    );
    expect(snap.pythonVersion).toBe("3.11.7");
    expect(snap.condaEnv).toBe("ml-train");
    expect(snap.virtualenv).toBeNull();
    expect(snap.modules).toEqual(["gcc/11.3.0", "cuda/12.4", "cudnn/8.9.2"]);
  });

  it("reduces a $VIRTUAL_ENV path to its basename and ignores leading profile noise", () => {
    const snap = parseSnapshot(
      withEnv(
        [
          "Welcome to the cluster!", // MOTD-style noise before the markers
          "@@PY Python 3.10.12",
          "@@CONDA ",
          "@@VENV /home/researcher/projects/qwen/.venv",
          "@@MODULE",
          "No modules loaded",
        ].join("\n"),
      ),
    );
    expect(snap.pythonVersion).toBe("3.10.12");
    expect(snap.condaEnv).toBeNull();
    expect(snap.virtualenv).toBe(".venv");
    expect(snap.modules).toBeNull();
  });

  it("treats a host without `module` (command not found) as no modules", () => {
    const snap = parseSnapshot(
      withEnv(
        ["@@PY Python 3.9.2", "@@CONDA ", "@@VENV ", "@@MODULE", "bash: module: command not found"].join("\n"),
      ),
    );
    expect(snap.pythonVersion).toBe("3.9.2");
    expect(snap.modules).toBeNull();
  });

  it("leaves every env field null when the block is absent (graceful degrade)", () => {
    const snap = parseSnapshot("###USER\ndeploy\n###OS\nLinux 6.5.0\n");
    expect(snap.pythonVersion).toBeNull();
    expect(snap.condaEnv).toBeNull();
    expect(snap.virtualenv).toBeNull();
    expect(snap.modules).toBeNull();
  });

  it("does not regress CUDA / driver parsing from the GPU banner", () => {
    const stdout = [
      "###USER",
      "researcher",
      "###GPU",
      "NVIDIA A100-SXM4-40GB, 40960, 535.183.01",
      "###GPUHDR",
      "| NVIDIA-SMI 535.183.01   Driver Version: 535.183.01   CUDA Version: 12.4 |",
    ].join("\n");
    const snap = parseSnapshot(stdout);
    expect(snap.cudaVersion).toBe("12.4");
    expect(snap.driverVersion).toBe("535.183.01");
    expect(snap.gpuCount).toBe(1);
  });
});
