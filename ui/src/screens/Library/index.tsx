// New for agent-router: the Library screen at `/library`, a hosted router's skills, commands and
// subagents (docs/library.md; docs/web-console-contracts.md, "Library"). Adding a version comes first,
// then every item at a glance, and the item opened by `?item=kind/name` under them: its file as Claude
// Code reads it, its files, every version with where it came from, what can be done to each, and how
// one differs from the live one. A local router keeps no Library, and the screen says so (LB-C1).
// How each item is used, and what the Library's skills add to every turn, are read apart from the list
// (LB-C14 to LB-C17).
import { useEffect, useState, type ChangeEvent } from 'react';
import { Link, useSearchParams } from 'react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Button, ContainedList, ContainedListItem, Select, SelectItem, TextInput } from '@carbon/react';
import { BarList } from '~/components/BarList';
import { EmptyState } from '~/components/EmptyState';
import { Page } from '~/components/Page';
import { PageHeader } from '~/components/PageHeader';
import { Sparkline } from '~/components/Sparkline';
import { useRead, type Read } from '~/data/held';
import { archive, libraryKeys, makeLive, readFile, readItem, readLibrary, readUsage, reject, syncNow, upload } from '~/data/library';
import { useAuth } from '~/data/settings';
import { refusalText } from '~/lib/errors';
import { POLL_MS, since } from '~/lib/freshness';
import { routes } from '~/lib/routes';
import { md } from '~/model/markdown';
import { ago, monthDay, projectName, UNRESOLVED, when } from '~/model/format';
import {
  ACT_WORDS,
  EMPTY,
  KINDS,
  KIND_WORDS,
  NOT_HERE,
  TITLE,
  actsOn,
  bodyOf,
  diffLines,
  isName,
  itemOrder,
  itemParam,
  listingWords,
  liveWords,
  machineWords,
  mainFile,
  nameFrom,
  packFolder,
  provenanceWords,
  publishedWords,
  sessionOf,
  sourceWords,
  stateWords,
  toldWords,
  typeOf,
  usedWords,
  type Act,
  type ItemRow,
  type Kind,
  type Usage,
  type Version,
} from '~/model/library';
import { isHosted } from '~/model/settings';
import { Markdown } from '~/screens/Sessions/Markdown';
import styles from './Library.module.scss';

/// `/library`. What kind of router this is decides what is drawn, from `/api/auth`'s `mode`; until
/// it has answered, neither is (LB-C1).
export default function Library() {
  // The screen names itself in the tab's title (SH-C7).
  useEffect(() => {
    document.title = TITLE;
  }, []);
  const auth = useAuth();
  return (
    <Page>
      <PageHeader title="Library" />
      {isHosted(auth.value) ? (
        <Hosted />
      ) : auth.value !== null ? (
        // A local router keeps no Library: nothing is read and nothing offered (LB-C1).
        <p className={styles.local}>{NOT_HERE}</p>
      ) : auth.unreachable ? (
        <EmptyState title="Could not read what kind of router this is">{refusalText(auth.error)}</EmptyState>
      ) : (
        <p className={styles.muted}>Asking the router what kind it is…</p>
      )}
    </Page>
  );
}

/// Usage is counted by day, so it is read as auth and accounts are, every 30 s (SH-C15).
const USAGE_MS = 30_000;

function Hosted() {
  // Every five seconds on the console's one clock, never retried quietly (LB-C13, SH-C15).
  const list = useRead({ queryKey: libraryKeys.list, queryFn: ({ signal }) => readLibrary(signal), refetchInterval: POLL_MS, retry: false, staleTime: 0 });
  const usage = useRead({ queryKey: libraryKeys.usage, queryFn: ({ signal }) => readUsage(signal), refetchInterval: USAGE_MS, retry: false, staleTime: 0 });
  const [params] = useSearchParams();
  const open = itemParam(params.get('item'));
  const items = [...(list.value ?? [])].sort(itemOrder);
  const failed = list.stale || list.unreachable ? refusalText(list.error) : null;
  return (
    <>
      <Page.Section title="Add to the Library">
        <Upload />
      </Page.Section>
      <Page.Section title="Items">
        <Cost usage={usage} />
        <SyncAll />
        {list.value !== null && failed !== null && (
          // Kept, never blanked, and marked with its age (LB-C13).
          <p className={styles.stale}>
            Showing what was read {since(list.at)}: the last read failed — {failed}
          </p>
        )}
        {list.value === null ? (
          list.unreachable ? (
            // A read that failed is not an empty Library (LB-C13).
            <EmptyState title="Could not read the Library">{failed}</EmptyState>
          ) : (
            <p className={styles.muted}>Reading the Library…</p>
          )
        ) : items.length === 0 ? (
          <p className={styles.none}>{EMPTY}</p>
        ) : (
          <ContainedList kind="on-page" size="lg" className={styles.list} label={`${items.length} in the Library`}>
            {items.map((row) => (
              <Row key={`${row.kind}/${row.name}`} row={row} open={open?.kind === row.kind && open.name === row.name} />
            ))}
          </ContainedList>
        )}
      </Page.Section>
      {open && <Item key={`${open.kind}/${open.name}`} kind={open.kind} name={open.name} usage={usage} />}
    </>
  );
}

/// One item at a glance: what it is, what it says it is for, its live version, where its versions
/// come from, what waits for approval, and whether it is archived (LB-C2).
function Row({ row, open }: { row: ItemRow; open: boolean }) {
  return (
    <ContainedListItem className={styles.row} data-item={`${row.kind}/${row.name}`} data-open={open ? '' : undefined}>
      <Link to={{ pathname: routes.library, search: `?item=${row.kind}/${row.name}` }} className={styles.open}>
        <span className={styles.kind}>{KIND_WORDS[row.kind]}</span> <span className={styles.name}>{row.name}</span>
      </Link>
      <span className={styles.description}>{row.description}</span>
      <span className={styles.facts}>
        <span data-live={row.live ? '' : undefined}>{liveWords(row)}</span>
        <span>{sourceWords(row.source)}</span>
        {row.pending > 0 && <span className={styles.waiting}>{row.pending} waiting for approval</span>}
        {row.archived_at !== null && <span className={styles.archived}>archived</span>}
        {/* Said only by a router that counts it: never "not used" for want of a count (LB-C15, LB-C17). */}
        {row.uses_30d !== undefined && (
          <span data-used={row.uses_30d} className={row.uses_30d === 0 ? styles.unused : undefined}>
            {usedWords(row.uses_30d)}
          </span>
        )}
      </span>
    </ContainedListItem>
  );
}

/// What the Library's skills add to every turn, and each one's share; usage that could not be read
/// says so, and never reads as nothing added (LB-C14, LB-C17).
function Cost({ usage }: { usage: Read<Usage> }) {
  if (usage.value === null) {
    return usage.unreachable ? (
      <p className={styles.stale} data-cost="failed">
        The Library’s usage could not be read: {refusalText(usage.error)}
      </p>
    ) : null;
  }
  const listing = usage.value.listing;
  const shares = [...(listing?.items ?? [])].sort((a, b) => b.tokens - a.tokens);
  return (
    <div className={styles.cost} data-cost="">
      <p className={styles.costLine}>{listingWords(listing)}</p>
      {shares.length > 0 && (
        <BarList
          label="What each skill adds to every turn"
          rows={shares.map((e) => ({ key: e.name, name: e.name, value: e.tokens, figure: `about ${e.tokens.toLocaleString()} tokens` }))}
        />
      )}
    </div>
  );
}

/// An item's use over the last 30 days, a bar per day, and its uses by machine and by project
/// (LB-C16). Usage that could not be read is never "not used" (LB-C17).
function Use({ kind, name, usage }: { kind: Kind; name: string; usage: Read<Usage> }) {
  if (usage.value === null) {
    return usage.unreachable ? (
      <p className={styles.stale} data-use="failed">
        Its usage could not be read: {refusalText(usage.error)}
      </p>
    ) : (
      <p className={styles.muted}>Reading its usage…</p>
    );
  }
  const mine = usage.value.items.find((u) => u.kind === kind && u.name === name);
  if (!mine || mine.uses_30d === 0) {
    const last = mine?.last_used_at ? `; last used ${when(mine.last_used_at)}` : '';
    return (
      <p className={styles.unused} data-use="none">
        Not used in the last 30 days{last}.
      </p>
    );
  }
  const days = usage.value.days.map(monthDay);
  return (
    <div className={styles.use} data-use="">
      <p>
        {usedWords(mine.uses_30d)}
        {mine.last_used_at ? `, last ${ago(mine.last_used_at)}` : ''}
      </p>
      <Sparkline label="Uses per day" buckets={days} series={[{ key: 'uses', label: 'uses', values: mine.daily, colour: '--ar-tok' }]} ends />
      <div className={styles.splits}>
        <div>
          <h4 className={styles.h4}>By machine</h4>
          <BarList label="Uses by machine" rows={mine.machines.map((m) => ({ key: m.id ?? '', name: machineWords(m), value: m.uses, figure: m.uses.toLocaleString() }))} />
        </div>
        <div>
          <h4 className={styles.h4}>By project</h4>
          <BarList
            label="Uses by project"
            rows={mine.projects.map((p) => ({
              key: p.project ?? UNRESOLVED,
              name: projectName(p.project),
              path: p.project ?? undefined,
              value: p.uses,
              figure: p.uses.toLocaleString(),
            }))}
          />
        </div>
      </div>
    </div>
  );
}

/// The item opened: how it is used, its file as Claude Code reads it, its files, its versions and
/// what can be done to each, and the archive (LB-C4 to LB-C8, LB-C16).
function Item({ kind, name, usage }: { kind: Kind; name: string; usage: Read<Usage> }) {
  const client = useQueryClient();
  const item = useRead({ queryKey: libraryKeys.item(kind, name), queryFn: ({ signal }) => readItem(kind, name, signal), retry: false, staleTime: 0 });
  const [file, setFile] = useState<string | null>(null);
  const [compare, setCompare] = useState<number | null>(null);
  const [said, setSaid] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const title = `${KIND_WORDS[kind]} ${name}`;
  const detail = item.value;
  if (detail === null) {
    return (
      <Page.Section title={title}>
        {item.unreachable ? <EmptyState title={`Could not read the ${kind} ${name}`}>{refusalText(item.error)}</EmptyState> : <p className={styles.muted}>Reading it…</p>}
      </Page.Section>
    );
  }
  const live = detail.versions.find((v) => v.state === 'live') ?? null;
  const shown = live ?? detail.versions[0] ?? null;
  // Each act is done once, and everything the Library shows is read again after it (LB-C6).
  const act = async (run: () => Promise<unknown>, done: string) => {
    setBusy(true);
    try {
      await run();
      setSaid(done);
    } catch (err) {
      setSaid(refusalText(err));
    } finally {
      setBusy(false);
      setConfirming(false);
      await client.invalidateQueries({ queryKey: libraryKeys.all });
    }
  };
  const decide = (a: Act, v: Version) =>
    act(
      () => (a === 'reject' ? reject(kind, name, v.number) : makeLive(kind, name, v.number)),
      a === 'reject' ? `Version ${v.number} is rejected.` : `Version ${v.number} is live: every connected machine is told now.`,
    );
  return (
    <Page.Section
      title={title}
      action={
        detail.archived_at === null ? (
          confirming ? (
            <span className={styles.confirm}>
              <Button kind="danger" size="sm" disabled={busy} onClick={() => act(() => archive(kind, name), `The ${kind} ${name} is archived: every machine takes it away.`)}>
                Take it off every machine
              </Button>
              <Button kind="ghost" size="sm" onClick={() => setConfirming(false)}>
                Keep it
              </Button>
            </span>
          ) : (
            // Asked once more before it reaches every machine (LB-C8).
            <Button kind="tertiary" size="sm" onClick={() => setConfirming(true)}>
              Archive
            </Button>
          )
        ) : undefined
      }
    >
      <p className={styles.description}>{detail.description}</p>
      <p className={styles.facts}>
        <span>{sourceWords(detail.source)}</span>
        {detail.archived_at !== null && <span className={styles.archived}>archived: on no machine until a version of it goes live again</span>}
      </p>
      {said && (
        <p role="status" className={styles.said}>
          {said}
        </p>
      )}
      <h3 className={styles.h3}>Use in the last 30 days</h3>
      <Use kind={kind} name={name} usage={usage} />
      {shown && <MainFile kind={kind} name={name} version={shown.number} />}

      <h3 className={styles.h3}>Files{shown ? ` of version ${shown.number}` : ''}</h3>
      <ul className={styles.files}>
        {detail.files.map((f) => (
          <li key={f.path} data-file={f.path}>
            <Button kind="ghost" size="sm" onClick={() => setFile(file === f.path ? null : f.path)}>
              {f.path}
            </Button>
            <span className={styles.muted}>
              {f.size} bytes{f.executable ? ', executable' : ''}
            </span>
          </li>
        ))}
      </ul>
      {file && shown && <FileText kind={kind} name={name} version={shown.number} path={file} />}

      <h3 className={styles.h3}>Versions</h3>
      <ul className={styles.versions}>
        {detail.versions.map((v) => {
          const session = sessionOf(v.provenance);
          return (
            <li key={v.number} data-version={v.number} data-state={v.state}>
              <span className={styles.number}>v{v.number}</span>
              <span className={styles.state}>{stateWords(v.state)}</span>
              <span>{provenanceWords(v.provenance)}</span>
              {session && <Link to={`${routes.sessions}?session=${encodeURIComponent(session)}`}>open the session</Link>}
              <span className={styles.muted}>{when(v.created_at)}</span>
              <span className={styles.acts}>
                {actsOn(v).map((a) => (
                  <Button key={a} kind={a === 'reject' ? 'danger--ghost' : 'ghost'} size="sm" disabled={busy} onClick={() => decide(a, v)}>
                    {ACT_WORDS[a]}
                  </Button>
                ))}
                {live && v.number !== live.number && (
                  <Button kind="ghost" size="sm" onClick={() => setCompare(compare === v.number ? null : v.number)}>
                    {compare === v.number ? 'Hide the changes' : 'Compare with the live one'}
                  </Button>
                )}
              </span>
            </li>
          );
        })}
      </ul>
      {compare !== null && live && <Compare kind={kind} name={name} from={live.number} to={compare} />}
    </Page.Section>
  );
}

/// The file Claude Code reads, without its frontmatter, as Markdown: a React tree, never HTML
/// (LB-C4, SE-C90).
function MainFile({ kind, name, version }: { kind: Kind; name: string; version: number }) {
  const path = mainFile(kind, name);
  const text = useQuery({ queryKey: libraryKeys.file(kind, name, version, path), queryFn: ({ signal }) => readFile(kind, name, version, path, signal), retry: false });
  if (text.isError) return <p className={styles.muted}>Could not read {path}: {refusalText(text.error)}</p>;
  if (text.data === undefined) return <p className={styles.muted}>Reading {path}…</p>;
  return (
    <div className={styles.main} data-main={path}>
      <Markdown blocks={md(bodyOf(text.data))} />
    </div>
  );
}

/// One file, as text (LB-C4).
function FileText({ kind, name, version, path }: { kind: Kind; name: string; version: number; path: string }) {
  const text = useQuery({ queryKey: libraryKeys.file(kind, name, version, path), queryFn: ({ signal }) => readFile(kind, name, version, path, signal), retry: false });
  return (
    <pre className={styles.text} data-text={path}>
      {text.isError ? refusalText(text.error) : (text.data ?? 'Reading…')}
    </pre>
  );
}

/// How a version's main file differs from the live one's, line by line (LB-C7).
function Compare({ kind, name, from, to }: { kind: Kind; name: string; from: number; to: number }) {
  const path = mainFile(kind, name);
  const before = useQuery({ queryKey: libraryKeys.file(kind, name, from, path), queryFn: ({ signal }) => readFile(kind, name, from, path, signal), retry: false });
  const after = useQuery({ queryKey: libraryKeys.file(kind, name, to, path), queryFn: ({ signal }) => readFile(kind, name, to, path, signal), retry: false });
  if (before.data === undefined || after.data === undefined) return <p className={styles.muted}>Reading both versions…</p>;
  const lines = diffLines(before.data, after.data);
  return (
    <div className={styles.compare} data-compare={`${from}..${to}`}>
      <p className={styles.muted}>
        {path}: version {to} against the live version {from}
      </p>
      <pre className={styles.diff}>
        {lines.map((l, i) => (
          <span key={i} data-diff={l.kind} className={styles[l.kind]}>
            {l.kind === 'added' ? '+ ' : l.kind === 'removed' ? '- ' : '  '}
            {l.text}
            {'\n'}
          </span>
        ))}
      </pre>
    </div>
  );
}

/// A file's bytes, whichever way this browser reads them.
function bytesOf(file: File): Promise<Uint8Array> {
  if (typeof file.arrayBuffer === 'function') return file.arrayBuffer().then((b) => new Uint8Array(b));
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(new Uint8Array(reader.result as ArrayBuffer));
    reader.onerror = () => reject(reader.error);
    reader.readAsArrayBuffer(file);
  });
}

/// Adding a version: a kind, a name, and a skill's folder or one file — a .zip, a .tar.gz or one
/// .md. A folder is packed here into the tar the router reads; the name is the folder's or the
/// file's unless one is typed (LB-C9).
function Upload() {
  const client = useQueryClient();
  const [kind, setKind] = useState<Kind>('skill');
  const [name, setName] = useState('');
  const [chosen, setChosen] = useState<{ label: string; body: Blob; type: string; suggested: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState<string | null>(null);
  const chooseFolder = async (e: ChangeEvent<HTMLInputElement>) => {
    const files = [...(e.target.files ?? [])];
    if (files.length === 0) return;
    const read = await Promise.all(files.map(async (f) => ({ path: f.webkitRelativePath || f.name, bytes: await bytesOf(f) })));
    const top = read[0]?.path.split('/')[0] ?? '';
    setKind('skill');
    setChosen({ label: `the folder ${top}`, body: new Blob([packFolder(read) as BlobPart]), type: 'application/x-tar', suggested: nameFrom(top) });
    setSaid(null);
  };
  const chooseFile = (e: ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0];
    if (!f) return;
    setChosen({ label: f.name, body: f, type: typeOf(f.name), suggested: nameFrom(f.name) });
    setSaid(null);
  };
  const wanted = name.trim() || chosen?.suggested || '';
  const send = async () => {
    if (!chosen) return;
    setBusy(true);
    try {
      const done = await upload(kind, wanted, chosen.body, chosen.type);
      setSaid(publishedWords(kind, wanted, done.unchanged, done.version));
      setChosen(null);
      setName('');
    } catch (err) {
      setSaid(refusalText(err));
    } finally {
      setBusy(false);
      await client.invalidateQueries({ queryKey: libraryKeys.all });
    }
  };
  return (
    <div className={styles.upload}>
      <Select id="library-kind" labelText="Kind" value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
        {KINDS.map((k) => (
          <SelectItem key={k} value={k} text={KIND_WORDS[k]} />
        ))}
      </Select>
      <div className={styles.picks}>
        <label className={styles.pick}>
          <span>Choose a skill’s folder</span>
          {/* A folder, as the browser lets one be chosen: every file in it, by its path. */}
          <input type="file" data-pick="folder" multiple onChange={chooseFolder} {...({ webkitdirectory: '' } as Record<string, string>)} />
        </label>
        <label className={styles.pick}>
          <span>Choose a file: a .zip, a .tar.gz or one .md</span>
          <input type="file" data-pick="file" accept=".zip,.tar,.gz,.tgz,.md" onChange={chooseFile} />
        </label>
      </div>
      {chosen && <p className={styles.chosen}>Chosen: {chosen.label}</p>}
      <TextInput
        id="library-name"
        labelText="Name"
        helperText="Lowercase letters and digits, with single hyphens between them"
        placeholder={chosen?.suggested ?? ''}
        value={name}
        onChange={(e) => setName(e.target.value)}
        invalid={wanted !== '' && !isName(wanted)}
        invalidText={`“${wanted}” cannot name an item: lowercase letters and digits, with single hyphens between them`}
      />
      <Button size="md" disabled={!chosen || !isName(wanted) || busy} onClick={send}>
        Publish
      </Button>
      {said && (
        <p role="status" className={styles.said}>
          {said}
        </p>
      )}
    </div>
  );
}

/// Tell every machine to sync now, and what each said (LB-C10).
function SyncAll() {
  const [busy, setBusy] = useState(false);
  const [told, setTold] = useState<string[] | null>(null);
  const sync = async () => {
    setBusy(true);
    try {
      const answer = await syncNow();
      setTold(answer.machines.length ? answer.machines.map(toldWords) : ['No machine has connected yet.']);
    } catch (err) {
      setTold([refusalText(err)]);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className={styles.sync}>
      <Button kind="tertiary" size="sm" disabled={busy} onClick={sync}>
        {busy ? 'Telling the machines…' : 'Sync every machine now'}
      </Button>
      {told && (
        <ul role="status" className={styles.told}>
          {told.map((t) => (
            <li key={t}>{t}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
