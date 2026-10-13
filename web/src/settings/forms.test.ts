import { describe, expect, test } from "vitest";
import {
  accessProblem,
  exitsProblem,
  formatCapacities,
  gitlabUrlProblem,
  hostNameProblem,
  normalizeGitlabUrl,
  oauthOutcome,
  parseCapacities,
  parseList,
  passwordProblem,
  retentionProblem,
  tokenPageUrl,
} from "./forms";

describe("forms", () => {
  test("passwords", () => {
    expect(passwordProblem("short", "short")).toMatch(/12/);
    expect(passwordProblem("a long enough one", "a long enough two")).toMatch(/match/);
    expect(passwordProblem("a long enough one", "a long enough one")).toBeNull();
    const longest = "é".repeat(1024);
    expect(passwordProblem(longest, longest)).toBeNull();
    expect(passwordProblem(longest + "x", longest + "x")).toMatch(/1024/);
  });

  test("lists split on lines and commas", () => {
    expect(parseList(" 10.0.0.0/8,\n192.168.1.5 \n\n")).toEqual(["10.0.0.0/8", "192.168.1.5"]);
  });

  test("access", () => {
    expect(accessProblem({ mode: "open" })).toBeNull();
    expect(accessProblem({ mode: "proxy", header: "x-forwarded-user", trusted: ["10.0.0.0/8", "::1"] })).toBeNull();
    expect(accessProblem({ mode: "proxy", header: "bad header", trusted: ["10.0.0.1"] })).toMatch(/header/);
    expect(accessProblem({ mode: "proxy", header: "x-u", trusted: [] })).toMatch(/trusted/);
    expect(accessProblem({ mode: "proxy", header: "x-u", trusted: ["proxy.local"] })).toMatch(/proxy\.local/);
  });

  test("exits", () => {
    expect(exitsProblem([{ label: "prod", stage: "deploy" }])).toBeNull();
    expect(exitsProblem([{ label: "", stage: "deploy" }])).toMatch(/name/);
    expect(exitsProblem([{ label: "a", stage: "x" }, { label: "a", name: "y" }])).toMatch(/twice/);
    expect(exitsProblem([{ label: "a" }])).toMatch(/stage or a job name/);
    expect(exitsProblem([{ label: "a", stage: "(" }])).toMatch(/pattern/);
  });

  test("capacities", () => {
    expect(parseCapacities("shared = 5\ndeploy=1\n")).toEqual({ shared: 5, deploy: 1 });
    expect(parseCapacities("")).toEqual({});
    expect(parseCapacities("shared")).toMatch(/shared/);
    expect(parseCapacities("shared=0")).toMatch(/shared/);
    expect(formatCapacities({ deploy: 1, shared: 5 })).toBe("deploy=1\nshared=5");
  });

  test("retention", () => {
    expect(retentionProblem(14)).toBeNull();
    expect(retentionProblem(0)).not.toBeNull();
    expect(retentionProblem(366)).not.toBeNull();
    expect(retentionProblem(1.5)).not.toBeNull();
  });
});

describe("gitlab", () => {
  test("addresses", () => {
    expect(gitlabUrlProblem("https://gitlab.example.com")).toBeNull();
    expect(gitlabUrlProblem("http://127.0.0.1:8929/gitlab/")).toBeNull();
    expect(gitlabUrlProblem("gitlab.example.com")).not.toBeNull();
    expect(gitlabUrlProblem("ftp://gitlab.example.com")).not.toBeNull();
    expect(gitlabUrlProblem("https://gitlab.example.com/?a=1")).not.toBeNull();
    expect(normalizeGitlabUrl(" https://gitlab.example.com/// ")).toBe("https://gitlab.example.com");
  });

  test("the token page is prefilled", () => {
    expect(tokenPageUrl("https://gitlab.example.com")).toBe("https://gitlab.example.com/-/user_settings/personal_access_tokens?name=lorvaq&scopes=read_api");
  });

  test("oauth outcomes come back in the hash", () => {
    expect(oauthOutcome("#/settings?gitlab=connected")).toMatch(/connected/i);
    expect(oauthOutcome("#/settings?gitlab=not_admin")).toMatch(/administrator/);
    expect(oauthOutcome("#/settings?gitlab=whatever")).toMatch(/did not finish/);
    expect(oauthOutcome("#/settings")).toBeNull();
  });
});

describe("hosts", () => {
  test("names", () => {
    for (const good of ["build-01", "a", "runner.example_2", " ci1 "]) expect(hostNameProblem(good)).toBeNull();
    for (const bad of ["", "-x", "Build", "a b", "x".repeat(41)]) expect(hostNameProblem(bad)).not.toBeNull();
  });
});
