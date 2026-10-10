#!/usr/bin/env python3
"""Local-only fake CLI: asserts the wire contract and never invokes a provider."""
# Diagnostics first: stderr is discarded by the caller, so errors and a stack dump of a
# start-up that hangs past 10 s go to stderr.log next to the script (the test prints it).
import faulthandler
import pathlib
import sys
_log = open(pathlib.Path(__file__).with_name('stderr.log'), 'a')
sys.stderr = _log
faulthandler.enable(_log)
faulthandler.dump_traceback_later(10, exit=False, file=_log)
import base64
import http.server
import json
import os
import pathlib
import sys
import socketserver
import sqlite3
import threading
import time
import urllib.parse

if sys.argv[1:] == ["--version"]:
    print("2.0.6")
    sys.exit(0)

root = pathlib.Path(__file__).parent
port = int(sys.argv[sys.argv.index('--port') + 1])
assert sys.argv[sys.argv.index('--hostname') + 1] == '127.0.0.1'
auth = 'Basic ' + base64.b64encode(('opencode:' + os.environ['OPENCODE_PASSWORD']).encode()).decode()
(root / 'environment.json').write_text(json.dumps({key: os.environ.get(key) for key in ['HOME', 'USERPROFILE', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'XDG_STATE_HOME', 'XDG_CACHE_HOME', 'OPENCODE_DB', 'OPENCODE_CONFIG_DIR', 'OPENCODE_CONFIG_PROJECT_DISABLE']}))
with sqlite3.connect(os.environ['OPENCODE_DB']) as database:
    database.execute('CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT)')
log = root / 'requests.jsonl'
state = {'id': 'ses_mock', 'projectID': 'project_mock', 'location': {'directory': str(root)}, 'permissions': [{'action': '*', 'resource': '*', 'effect': 'allow'}], 'messages': [], 'pending': [], 'turn': 0}

class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *args): pass
    def send(self, value, code=200):
        raw = json.dumps(value).encode()
        self.send_response(code)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)
    def authorize(self):
        if self.headers.get('Authorization') != auth:
            self.send({'error': 'unauthorized'}, 401)
            return False
        return True
    def do_GET(self):
        if not self.authorize(): return
        path = urllib.parse.urlparse(self.path).path
        if path == '/api/info': self.send({'version': '2.0.6'})
        elif path == '/openapi.json': self.send(json.loads((root / 'contract.json').read_text()))
        elif path == '/api/model': self.send({'data': [{'providerID': 'mock', 'id': 'model', 'name': 'Mock', 'enabled': True, 'status': 'active', 'variants': [{'id': 'high'}]}]})
        elif path == '/api/permission/saved': self.send({'data': []})
        elif path == '/api/session/ses_mock': self.send({'data': {k: v for k, v in state.items() if k in ['id', 'projectID', 'location', 'permissions']}})
        elif path == '/api/session/ses_mock/permission': self.send({'data': state['pending']})
        elif path == '/api/session/ses_mock/message': self.send({'data': list(reversed(state['messages'])), 'cursor': {}})
        elif path == '/api/event':
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            try:
                while True:
                    self.wfile.write(b'data: {"type":"heartbeat"}\n\n')
                    self.wfile.flush()
                    time.sleep(0.1)
            except (BrokenPipeError, ConnectionResetError): pass
        else: self.send({'error': 'unexpected path'}, 404)
    def do_PATCH(self):
        if not self.authorize(): return
        body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))) or '{}')
        assert self.path == '/api/session/ses_mock'
        with log.open('a') as f: f.write(json.dumps({'path': self.path, 'body': body}) + '\n')
        state['permissions'] = body['permissions']
        self.send(None)
    def do_POST(self):
        if not self.authorize(): return
        body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))) or '{}')
        with log.open('a') as f: f.write(json.dumps({'path': self.path, 'body': body}) + '\n')
        if self.path == '/api/session':
            state['location'] = body['location']
            assert body['agent'] == 'colm'
            self.send({'data': {'id': state['id'], 'projectID': state['projectID'], 'location': state['location']}})
        elif self.path.endswith('/agent'):
            assert body == {'agent': 'colm'}
            self.send(None)
        elif self.path.endswith('/model'):
            assert body['model']['providerID'] == 'mock'
            assert body['model']['id'] == 'model'
            self.send(None)
        elif self.path.endswith('/prompt'):
            state['turn'] += 1
            state['messages'].append({'id': body['id'], 'type': 'user', 'text': body['text']})
            if body['text'] != 'cancel':
                state['pending'] = [{'id': 'per_mock', 'sessionID': state['id'], 'action': 'colm_list_cases' if body['text'] == 'colm' else 'shell', 'resources': ['echo hello'], 'metadata': {'command': 'echo hello'}}]
            self.send({'data': {'id': body['id']}})
        elif self.path.endswith('/permission/per_mock/reply'):
            assert body['decision'] in ['once', 'reject']
            state['pending'] = []
            state['messages'] += [
                {'id': 'msg_answer' + str(state['turn']), 'type': 'assistant', 'finish': 'stop', 'tokens': {'input': 3, 'output': 4}, 'content': [
                    {'type': 'reasoning', 'text': 'check'},
                    {'type': 'tool', 'id': 'tool_' + str(state['turn']), 'name': 'shell', 'state': {'status': 'completed', 'input': {'command': 'echo hello'}, 'content': [{'type': 'text', 'text': 'hello'}]}},
                    {'type': 'text', 'text': 'done'}]},
                {'id': 'msg_idle' + str(state['turn']), 'type': 'idle', 'outcome': 'succeeded'}]
            self.send(None)
        elif self.path.endswith('/interrupt'):
            self.send({'interrupted': True})
        else: self.send({'error': 'unexpected path'}, 404)

class Server(http.server.ThreadingHTTPServer):
    # HTTPServer.server_bind calls socket.getfqdn(), a reverse DNS lookup of 127.0.0.1 that
    # stalls past the caller's 15 s start-up limit on the macOS CI runners. Bind without it.
    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]

server = Server(('127.0.0.1', port), Handler)
print(json.dumps({'url': 'http://127.0.0.1:' + str(server.server_port)}), flush=True)
faulthandler.cancel_dump_traceback_later()
server.serve_forever()
