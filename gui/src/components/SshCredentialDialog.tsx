import { useEffect, useRef, useState } from 'react';
import { useApp } from '../lib/app-state';
import { usePageContract } from '../lib/guide';
import { hasBackend, invoke, invokeJson, listen } from '../lib/ipc';
import { Button, Dialog, Input, Notice } from './ui';

type AuthRequest = {
  request_id: string;
  operation_host: string;
  prompt: string;
  kind: 'passphrase' | 'password';
};
type AuthSnapshot = { revision: number; requests: AuthRequest[] };

export default function SshCredentialDialog() {
  const { state, notify } = useApp();
  const en = state.language === 'en';
  const [snapshot, setSnapshot] = useState<AuthSnapshot>({ revision: -1, requests: [] });
  useEffect(() => {
    if (!hasBackend()) return;
    let active = true;
    let unlisten: (() => void) | undefined;
    const apply = (next: AuthSnapshot) => {
      if (active) setSnapshot((old) => (next.revision > old.revision ? next : old));
    };
    void (async () => {
      unlisten = await listen<AuthSnapshot>('ssh-auth://changed', apply);
      if (!active) {
        unlisten();
        return;
      }
      apply(await invokeJson<AuthSnapshot>('remote_auth_pending'));
    })().catch(() => {
      if (active)
        notify(
          en ? 'Unable to receive SSH authentication requests.' : '无法接收 SSH 认证请求。',
          true,
        );
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [en, notify]);
  const request = snapshot.requests[0];
  usePageContract(
    {
      title: en ? 'SSH authentication' : 'SSH 认证',
      step: 'ssh-auth',
      modal: true,
      fields: [],
      actions: [],
    },
    !!request,
  );
  return request ? (
    <CredentialPrompt
      key={request.request_id}
      request={request}
      en={en}
      onAnswered={() =>
        setSnapshot((old) => ({
          ...old,
          requests: old.requests.filter((item) => item.request_id !== request.request_id),
        }))
      }
    />
  ) : null;
}

function CredentialPrompt({
  request,
  en,
  onAnswered,
}: {
  request: AuthRequest;
  en: boolean;
  onAnswered: () => void;
}) {
  const [secret, setSecret] = useState('');
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const submitting = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const respond = async (value: string | null) => {
    if (submitting.current) return;
    submitting.current = true;
    setBusy(true);
    setFailed(false);
    setSecret('');
    try {
      await invoke('remote_auth_respond', { requestId: request.request_id, value });
      if (mounted.current) onAnswered();
    } catch {
      if (mounted.current) setFailed(true);
    } finally {
      submitting.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  const label =
    request.kind === 'passphrase'
      ? en
        ? 'Key passphrase'
        : '私钥口令'
      : en
        ? 'Login password'
        : '登录密码';
  return (
    <Dialog
      open
      showClose={false}
      title={en ? 'SSH authentication' : 'SSH 认证'}
      description={
        en
          ? 'Enter the requested credential to continue this operation.'
          : '输入所需凭据以继续当前操作。'
      }
      onOpenChange={(open) => {
        if (!open) void respond(null);
      }}
    >
      <p>{request.operation_host}</p>
      <p>{request.prompt}</p>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void respond(secret);
        }}
      >
        <label>
          {label}
          <Input
            type="password"
            autoComplete="off"
            autoFocus
            value={secret}
            disabled={busy}
            onChange={(event) => setSecret(event.target.value)}
          />
        </label>
        {failed && (
          <Notice tone="error">
            {en ? 'Could not send the response. Please retry.' : '未能发送，请重新输入后重试。'}
          </Notice>
        )}
        <div className="button-row">
          <Button type="button" disabled={busy} onClick={() => void respond(null)}>
            {en ? 'Cancel operation' : '取消操作'}
          </Button>
          <Button type="submit" variant="primary" disabled={busy}>
            {en ? 'Continue' : '继续'}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
