import { useEffect, useState } from "react";
import { Button, Textarea } from "@mantine/core";
import type { RunnerSettings } from "../../models/topology";
import { useLocale } from "../../i18n/locale";
import { useProduct } from "../../i18n/product";

export function ExtensionPathsEditor({ settings, disabled, onSave }: {
  settings: RunnerSettings; disabled: boolean; onSave: (paths: RunnerSettings["paths"]) => Promise<boolean>;
}) {
  const { t } = useLocale(); const p = useProduct();
  const [instructions, setInstructions] = useState(settings.paths.instruction_files.join("\n"));
  const [skills, setSkills] = useState(settings.paths.skill_roots.join("\n"));
  useEffect(() => { setInstructions(settings.paths.instruction_files.join("\n")); setSkills(settings.paths.skill_roots.join("\n")); }, [settings]);
  return <form className="extension-editor" onSubmit={event => {
    event.preventDefault(); if (disabled) return;
    const lines = (value: string) => value.split(/\r?\n/).map(line => line.trim()).filter(Boolean);
    void onSave({ instruction_files: lines(instructions), skill_roots: lines(skills) });
  }}>
    <Textarea id="instruction-files" label={t("extensions.instructionFiles")} rows={3} value={instructions} onChange={event => setInstructions(event.currentTarget.value)} disabled={disabled} spellCheck={false} />
    <Textarea id="skill-roots" label={t("extensions.skillRoots")} rows={3} value={skills} onChange={event => setSkills(event.currentTarget.value)} disabled={disabled} spellCheck={false} />
    <Button className="primary-button" type="submit" data-webcodex-action="save-runner-settings" disabled={disabled}>{p("save")}</Button>
  </form>;
}
