import { useRef, useState, type FormEvent } from "react";
import { desktopApi } from "../../lib/desktop-api";
import type { DesktopState } from "../../models/topology";
import type { McpProviderProfile, McpProviderRequest } from "../../models/connections-tools";
import { useProduct } from "../../i18n/product";
import { useConnectionsTools } from "../../i18n/connections-tools";
import { WorkspaceDialog } from "../workspace/WorkspaceDialog";

type EnvField = { row: number; name: string; value: string; savedName: string | null };
export function McpProviderEditor({ profile, revision, onState, onClose }: { profile: McpProviderProfile | null; revision: number; onState: (state: DesktopState) => void; onClose: () => void }) {
  const p = useProduct(); const c = useConnectionsTools();
  const [name, setName] = useState(profile?.name ?? "");
  const [command, setCommand] = useState(profile?.command ?? "");
  const [args, setArgs] = useState(JSON.stringify(profile?.args ?? []));
  const [cwd, setCwd] = useState(profile?.cwd ?? "");
  const [enabled, setEnabled] = useState(profile?.enabled ?? true);
  const [env, setEnv] = useState<EnvField[]>(() => (profile?.env_keys ?? []).map((name, row) => ({ row, name, value: "", savedName: name })));
  const nextRow = useRef(env.length); const submitting = useRef(false);
  const [busy, setBusy] = useState(false); const [error, setError] = useState<"invalidFields" | "operationFailed" | null>(null);
  const changeEnv = (row: number, patch: Partial<EnvField>) => setEnv(fields => fields.map(field => field.row === row ? { ...field, ...patch } : field));
  const submit = async (event: FormEvent) => {
    event.preventDefault(); if (submitting.current) return;
    let request: McpProviderRequest;
    try {
      const parsed: unknown = JSON.parse(args);
      if (!Array.isArray(parsed) || parsed.length > 64 || parsed.some(value => typeof value !== "string" || value.includes("\0") || value.length > 4096)) throw new Error("invalid");
      const keys = new Set<string>();
      const entries = env.map(field => {
        const key = field.name.trim(); const normalized = key.toUpperCase();
        if (!/^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(key) || normalized.startsWith("WEBCODEX_") || normalized === "AUTHORIZATION" || keys.has(normalized)) throw new Error("invalid");
        keys.add(normalized);
        return [key, !field.value && field.savedName === key ? null : field.value] as const;
      });
      request = { id: profile?.id ?? null, expected_revision: revision, name: name.trim(), command: command.trim(), args: parsed as string[], cwd: cwd.trim() || null, enabled, env: Object.fromEntries(entries) };
    } catch { setError("invalidFields"); return; }
    submitting.current = true; setBusy(true); setError(null);
    setEnv(fields => fields.map(field => ({ ...field, value: "" })));
    try { onState(await desktopApi.saveMcpProvider(request)); onClose(); }
    catch { setError("operationFailed"); }
    finally { request.env = {}; submitting.current = false; setBusy(false); }
  };
  return <WorkspaceDialog title={profile ? `${p("edit")} ${profile.name}` : c("addMcpProvider")} onClose={onClose} busy={busy}>
    <form className="profile-editor" onSubmit={event => void submit(event)} aria-busy={busy}>
      <div className="field-group"><label htmlFor="mcp-provider-name">{c("providerName")}</label><input id="mcp-provider-name" value={name} onChange={event => setName(event.target.value)} maxLength={128} required disabled={busy} autoFocus /></div>
      <div className="field-group"><label htmlFor="mcp-provider-command">{c("command")}</label><input id="mcp-provider-command" value={command} onChange={event => setCommand(event.target.value)} maxLength={1024} required disabled={busy} spellCheck={false} placeholder="npx" /></div>
      <div className="field-group"><label htmlFor="mcp-provider-arguments">{c("arguments")}</label><textarea id="mcp-provider-arguments" value={args} onChange={event => setArgs(event.target.value)} maxLength={65536} required rows={3} disabled={busy} spellCheck={false} aria-describedby="mcp-arguments-help" /><small id="mcp-arguments-help">{c("argsHelp")}</small></div>
      <label className="profile-checkbox" htmlFor="mcp-provider-enabled"><input id="mcp-provider-enabled" type="checkbox" checked={enabled} onChange={event => setEnabled(event.target.checked)} disabled={busy} />{c("enabled")}</label>
      <fieldset className="mcp-environment" disabled={busy}><legend>{c("environment")}</legend>
        {env.map(field => <div className="mcp-environment-row" key={field.row}>
          <div className="field-group"><label htmlFor={`mcp-env-name-${field.row}`}>{c("envName")} {field.row + 1}</label><input id={`mcp-env-name-${field.row}`} value={field.name} onChange={event => changeEnv(field.row, { name: event.target.value })} maxLength={128} required spellCheck={false} /></div>
          <div className="field-group"><label htmlFor={`mcp-env-value-${field.row}`}>{c("envValue")} {field.name || field.row + 1}</label><input id={`mcp-env-value-${field.row}`} type="password" value={field.value} onChange={event => changeEnv(field.row, { value: event.target.value })} maxLength={8192} autoComplete="new-password" spellCheck={false} aria-describedby={field.savedName ? `mcp-env-retained-${field.row}` : undefined} />{field.savedName && <small id={`mcp-env-retained-${field.row}`}>{c("keepCredential")}</small>}</div>
          <button type="button" className="text-button" aria-label={`${c("remove")} ${field.name || field.row + 1}`} onClick={() => setEnv(fields => fields.filter(value => value.row !== field.row))}>{c("remove")}</button>
        </div>)}
        <button type="button" className="secondary-button" disabled={env.length >= 64} onClick={() => { const row = nextRow.current++; setEnv(fields => [...fields, { row, name: "", value: "", savedName: null }]); }}>{c("addEnvironment")}</button>
      </fieldset>
      <details><summary>{p("advanced")}</summary><div className="field-group"><label htmlFor="mcp-provider-cwd">{c("workingDirectory")}</label><input id="mcp-provider-cwd" value={cwd} onChange={event => setCwd(event.target.value)} maxLength={4096} disabled={busy} spellCheck={false} /></div></details>
      {error && <p role="alert" className="workspace-notice">{c(error)}</p>}
      <div className="connection-actions"><button className="primary-button" type="submit" disabled={busy}>{p("save")}</button><button className="secondary-button" type="button" disabled={busy} onClick={onClose}>{p("cancel")}</button></div>
    </form>
  </WorkspaceDialog>;
}
