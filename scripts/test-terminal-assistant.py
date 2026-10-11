#!/usr/bin/env python3
"""End-to-end terminal checks against a local SSE fixture, without paid models or SSH."""
import argparse
import http.server
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import tempfile
import threading
import time


class Fixture(http.server.BaseHTTPRequestHandler):
    requests = []
    mode = "plain"

    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.requests.append(body)
        if self.mode == "error":
            self.send_error(503, "fixture unavailable")
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        tool = self.mode in ("mkdir", "resources") and body["messages"][-1]["role"] != "tool"
        if self.mode in ("poll", "budget"):
            tool = bool(body.get("tools"))
        if tool:
            if self.mode in ("poll", "budget"):
                name = "path_info"
                args = {"path": "." if self.mode == "poll" else f"missing-{len(self.requests)}"}
            else:
                name = "create_directory" if self.mode == "mkdir" else "remote_workspace_resources"
                args = {"path": "approved-output"} if self.mode == "mkdir" else {}
            delta = {"tool_calls": [{"index": 0, "id": "fixture-call", "type": "function",
                                    "function": {"name": name, "arguments": json.dumps(args)}}]}
        else:
            delta = {"content": "模拟回答完成"}
        chunk = {"choices": [{"index": 0, "delta": delta,
                              "finish_reason": "tool_calls" if tool else "stop"}]}
        self.wfile.write(("data: " + json.dumps(chunk) + "\n\ndata: [DONE]\n\n").encode())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent", type=Path, required=True)
    parser.add_argument("--cli", type=Path, required=True)
    opts = parser.parse_args()
    agent, cli = str(opts.agent.resolve()), str(opts.cli.resolve())
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    checks = []
    try:
        with tempfile.TemporaryDirectory(prefix="colm-terminal-e2e-") as temp:
            root = Path(temp)
            data = root / "state"
            env = dict(os.environ, COLM_AGENT_API_KEY="local-fixture-key")
            base = [agent, "chat", "--data-dir", str(data), "--cli", cli,
                    "--project-root", temp, "--base-url", f"http://127.0.0.1:{server.server_port}/v1",
                    "--provider-id", "custom", "--model", "fixture"]

            def run(args, success):
                result = subprocess.run(args, env=env, cwd=temp, capture_output=True, text=True, timeout=20)
                assert (result.returncode == 0) == success, (result.returncode, result.stdout, result.stderr)
                return result

            result = run(base + ["--message", "first fixture turn"], True)
            assert "模拟回答完成" in result.stdout
            assert "local-fixture-key" not in (data / "config.json").read_text()
            checks.append("streaming and nonsecret configuration")
            sessions = json.loads(run([agent, "--data-dir", str(data), "--list-sessions"], True).stdout)
            run(base + ["--resume", sessions[0]["id"], "--message", "second fixture turn"], True)
            users = [m["content"] for m in Fixture.requests[-1]["messages"] if m["role"] == "user"]
            assert users == ["first fixture turn", "second fixture turn"], users
            before = len(Fixture.requests)
            run(base + ["--resume", "missing-session", "--message", "must not send"], False)
            assert len(Fixture.requests) == before
            checks.append("saved context and missing-session rejection")

            Fixture.mode = "mkdir"
            run(base + ["--message", "create output directory"], False)
            assert not (root / "approved-output").exists()
            checks.append("noninteractive write denied")

            Fixture.mode = "resources"
            settings = root / "servers.json"
            settings.write_text(json.dumps({"servers": [{"host": "fixture-host", "root": "/safe/colm",
                                                        "username": "private-user", "auth": "key",
                                                        "identity_file": "/private/key"}]}))
            result = run(base + ["--servers", str(settings), "--message", "list server resources"], True)
            assert "fixture-host" in result.stdout
            assert "private-user" not in result.stdout and "/private/key" not in result.stdout
            names = {t["function"]["name"] for t in Fixture.requests[-1]["tools"]}
            assert "remote_workspace_submit" in names and not any(n.startswith("ui_") for n in names)
            checks.append("headless remote resources and credential filtering")

            for mode, generations in [("poll", 4), ("budget", 24)]:
                Fixture.mode = mode
                before = len(Fixture.requests)
                result = run(base + ["--message", f"{mode} fixture turn"], True)
                requests = Fixture.requests[before:]
                assert len(requests) == generations, (mode, len(requests))
                assert all(r.get("tools") for r in requests[:-1])
                assert "tools" not in requests[-1]
                assert "模拟回答完成" in result.stdout
                assert "[Turn execution limit]" in requests[-1]["messages"][-1]["content"]
                saved = list((data / "sessions").glob("*/messages.jsonl"))
                matching = [[json.loads(line) for line in path.read_text().splitlines()] for path in saved]
                history = next(rows for rows in matching if any(r.get("role") == "user" and r["content"] == f"{mode} fixture turn" for r in rows))
                assert not any(r.get("role") == "user" and "[Turn execution limit]" in r["content"] for r in history)
                assert history[-1]["role"] == "assistant" and not history[-1].get("tool_calls")
                checks.append(f"{mode} loop finishes with visible answer and resumable history")

            observations = list((data / "experiences").glob("*.json"))
            assert observations
            observed = [json.loads(path.read_text()) for path in observations]
            assert any(row["title"] == "repeated_read_queries: execution_limit" and row["status"] == "candidate" for row in observed)
            assert not any("local-fixture-key" in path.read_text() for path in observations)
            Fixture.mode = "plain"
            run(base + ["--message", "继续等待运行"], True)
            memory = [m["content"] for m in Fixture.requests[-1]["messages"] if m["role"] == "system" and "[Related experience records:" in m["content"]]
            assert len(memory) == 1 and "repeated_read_queries: execution_limit" in memory[0]
            saved = list((data / "sessions").glob("*/messages.jsonl"))
            assert not any("[Related experience records:" in path.read_text() for path in saved)
            names = {t["function"]["name"] for t in Fixture.requests[-1]["tools"]}
            assert {"experience_search", "experience_record", "experience_revoke"} <= names
            checks.append("experience observations reused across chats without persisting injected context")

            Fixture.mode = "error"
            run(base + ["--message", "fail fixture"], False)
            checks.append("provider failure exit status")

            if os.name == "posix":
                import termios
                master, slave = os.openpty()
                Fixture.mode = "mkdir"
                child = subprocess.Popen(base, env=env, cwd=temp, stdin=slave, stdout=slave, stderr=slave)
                output = bytearray()

                def until(marker):
                    deadline = time.monotonic() + 15
                    while marker not in output:
                        assert time.monotonic() < deadline, output.decode(errors="replace")
                        if select.select([master], [], [], 0.1)[0]:
                            output.extend(os.read(master, 65536))

                try:
                    until(b"/help for commands")
                    os.write(master, b"create output directory\n")
                    until(b"[y/n]")
                    os.write(master, b"y\n")
                    until("模拟回答完成".encode())
                    assert (root / "approved-output").is_dir()
                    os.kill(child.pid, signal.SIGINT)
                    child.wait(timeout=10)
                    assert termios.tcgetattr(slave)[3] & termios.ECHO
                    checks.append("PTY approval and SIGINT cleanup")
                finally:
                    if child.poll() is None:
                        child.kill()
                        child.wait()
                    os.close(master)
                    os.close(slave)
            print(json.dumps({"checks": checks, "passed": len(checks)}, ensure_ascii=False, indent=2))
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
