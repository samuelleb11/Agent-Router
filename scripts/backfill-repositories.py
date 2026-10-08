#!/usr/bin/env python3
"""Place past sessions under their repository, by hand, from the machine they ran on.

A machine's agent reports the sessions it lists, with each directory's git remotes, every
minute (src/places.rs), and from then on a session's project is its repository. Sessions the
router recorded before that, or whose transcript is gone, keep their directory as their
project. This script is the owner's way to place those, one machine at a time; the product
never does it on its own.

On the machine, it reads the agent's own file (~/.agent-router/agent.json: the router, its
password, this machine's id and label), takes the router's projects that are still a
directory, keeps the ones that are a git checkout here, and for each lists the sessions no
machine has placed yet. With --apply it posts them, with the directory's remotes, to the
router as this machine's report (POST /api/machines/places), which is what the agent sends.
A remote's credentials are taken out before anything leaves the machine.

    python3 -I scripts/backfill-repositories.py            # what it would place
    python3 -I scripts/backfill-repositories.py --apply    # place it
    python3 -I scripts/backfill-repositories.py --self-test
"""

import argparse
import base64
import json
import os
import subprocess
import sys
import urllib.parse
import urllib.request

# The router's session list stops at this many (src/api.rs, sessions_of).
PAGE = 200


def strip_credentials(url):
    """The URL without any `user[:password]@`, as src/repository.rs strips it."""
    url = url.strip()
    if "://" in url:
        scheme, rest = url.split("://", 1)
        authority, slash, tail = rest.partition("/")
        host = authority.rsplit("@", 1)[-1]
        return f"{scheme}://{host}{slash}{tail}"
    left, colon, path = url.partition(":")
    scp_like = colon and left and "/" not in left and "\\" not in left and not (len(left) == 1 and left.isalpha())
    if scp_like:
        return f"{left.rsplit('@', 1)[-1]}:{path}"
    return url


def parse_remotes(text):
    """`git config --get-regexp '^remote\\..*\\.url$'`: one `remote.<name>.url <url>` a line."""
    remotes = []
    for line in text.splitlines():
        key, _, url = line.partition(" ")
        if key.startswith("remote.") and key.endswith(".url") and url:
            remotes.append({"name": key[len("remote."):-len(".url")], "url": strip_credentials(url)})
    return remotes


def remotes_of(directory):
    """A directory's git remotes, or none when it is not a checkout here."""
    if not os.path.isdir(directory):
        return []
    try:
        out = subprocess.run(
            ["git", "-C", directory, "config", "--get-regexp", r"^remote\..*\.url$"],
            capture_output=True, text=True, timeout=10, check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return []
    return parse_remotes(out.stdout)


def unplaced_sessions(fetch, project):
    """Every session of a project no machine has placed, past the list's 200-row cap.

    `fetch(params)` answers GET /api/sessions. The list is the most recently seen first, so
    each next page asks for what started before the oldest of this page's last requests
    (`until`), and the ids seen are kept, since a long session can be on two pages."""
    ids, until = {}, None
    while True:
        params = {"project": project}
        if until is not None:
            params["until"] = str(until)
        page = fetch(params)
        for s in page:
            if not s.get("machine"):
                ids[s["id"]] = s
        if len(page) < PAGE:
            return list(ids.values())
        oldest = min(s["last_request_at"] for s in page) + 1
        until = oldest if until is None or oldest < until else until - 1


class Router:
    def __init__(self, server, password):
        self.server = server.rstrip("/")
        self.auth = "Basic " + base64.b64encode(f"x:{password}".encode()).decode()

    def _send(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.server + path, data=data, method=method)
        req.add_header("Authorization", self.auth)
        if data is not None:
            req.add_header("Content-Type", "application/json")
        with urllib.request.urlopen(req, timeout=60) as res:
            return json.load(res)

    def get(self, path, params=None):
        query = "?" + urllib.parse.urlencode(params) if params else ""
        return self._send("GET", path + query)

    def post(self, path, body):
        return self._send("POST", path, body)


def plan(router, remotes_of=remotes_of):
    """What this machine would place: (directory, its remotes, its sessions no machine placed)."""
    out = []
    for p in router.get("/api/projects"):
        directory = p.get("project")
        if p.get("repository") or not directory:
            continue
        remotes = remotes_of(directory)
        if not remotes:
            continue
        sessions = unplaced_sessions(lambda params: router.get("/api/sessions", params), directory)
        if sessions:
            out.append((directory, remotes, sessions))
    return out


def report(agent, directory, remotes, sessions):
    """This machine's report for one directory, as the agent posts it."""
    return {
        "machine": agent["machine"],
        "label": agent.get("label"),
        "platform": sys.platform,
        "sessions": [{"session": s["id"], "cwd": directory} for s in sessions],
        "directories": [{"cwd": directory, "remotes": remotes}],
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--apply", action="store_true", help="post the reports; without it, only say what they would place")
    ap.add_argument("--agent-json", default=os.path.expanduser("~/.agent-router/agent.json"))
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    with open(args.agent_json) as f:
        agent = json.load(f)
    router = Router(agent["server"], agent["password"])
    found = plan(router)
    if not found:
        print("Nothing to place from this machine.")
        return 0
    for directory, remotes, sessions in found:
        names = ", ".join(f"{r['name']} {r['url']}" for r in remotes)
        print(f"{directory}: {len(sessions)} session(s); remotes: {names}")
        if args.apply:
            answer = router.post("/api/machines/places", report(agent, directory, remotes, sessions))
            print(f"  placed {answer.get('placed')}, under a repository {answer.get('repositories')}")
    if not args.apply:
        print("Dry run: run again with --apply to place them.")
    return 0


def self_test():
    assert strip_credentials("https://oauth2:glpat-x@gitlab.example.com/acme/api.git") == "https://gitlab.example.com/acme/api.git"
    assert strip_credentials("https://ghp_x@github.com/o/r") == "https://github.com/o/r"
    assert strip_credentials("git@gitlab.example.com:acme/api.git") == "gitlab.example.com:acme/api.git"
    assert strip_credentials("/srv/git/api.git") == "/srv/git/api.git"
    assert strip_credentials("C:/src/api") == "C:/src/api"
    assert parse_remotes("remote.origin.url https://u:p@h.example/a/b.git\nremote.my.fork.url git@h.example:me/b.git\nnoise\n") == [
        {"name": "origin", "url": "https://h.example/a/b.git"},
        {"name": "my.fork", "url": "h.example:me/b.git"},
    ]

    # 450 sessions in one directory, one request each a second apart, newest first, three of
    # them already placed by a machine: every unplaced one is found, past the 200-row cap.
    sessions = [{"id": f"s-{i:03}", "last_request_at": 1_000 + i, "machine": {"id": "m"} if i % 150 == 0 else None} for i in range(450)]
    asked = []

    def fetch(params):
        asked.append(params)
        until = int(params.get("until", 10**12))
        within = [s for s in sessions if s["last_request_at"] < until]
        return sorted(within, key=lambda s: -s["last_request_at"])[:PAGE]

    found = unplaced_sessions(fetch, "/code/x")
    assert sorted(s["id"] for s in found) == sorted(s["id"] for s in sessions if not s["machine"]), "every unplaced session"
    assert len(asked) >= 3 and all(p["project"] == "/code/x" for p in asked), asked

    # Only a directory still keyed by itself, and a checkout here, is planned.
    class Fake:
        def get(self, path, params=None):
            if path == "/api/projects":
                return [
                    {"project": "gitlab.example.com/acme/done", "repository": "gitlab.example.com/acme/done"},
                    {"project": "/code/x", "repository": None},
                    {"project": "/code/not-a-checkout", "repository": None},
                    {"project": None, "repository": None},
                ]
            return fetch(params)

    remotes = {"/code/x": [{"name": "origin", "url": "https://gitlab.example.com/acme/x.git"}]}
    planned = plan(Fake(), remotes_of=lambda d: remotes.get(d, []))
    assert [d for d, _, _ in planned] == ["/code/x"], planned
    body = report({"machine": "mac", "label": "SYSOPs-Mac-mini"}, *planned[0])
    assert body["machine"] == "mac" and body["directories"] == [{"cwd": "/code/x", "remotes": remotes["/code/x"]}]
    assert len(body["sessions"]) == 447 and all(s["cwd"] == "/code/x" for s in body["sessions"])
    print("self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
