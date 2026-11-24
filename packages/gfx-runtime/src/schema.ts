export type PropFieldKind = "string" | "number" | "boolean" | "color";

export interface PropFieldDescriptor {
  key: string;
  label: string;
  kind: PropFieldKind;
  value: string | number | boolean;
}

const HEX_COLOR_RE = /^#[0-9a-fA-F]{6}$/;

function stringifyPropValue(val: unknown): string {
  if (val === undefined || val === null) return "";
  if (typeof val === "object") {
    try {
      return JSON.stringify(val);
    } catch {
      return "";
    }
  }
  return String(val);
}

function parseBooleanProp(val: unknown): boolean {
  if (typeof val === "boolean") return val;
  if (typeof val === "string") {
    const normalized = val.trim().toLowerCase();
    if (normalized === "" || normalized === "false" || normalized === "0") {
      return false;
    }
    return true;
  }
  return Boolean(val ?? false);
}

export function extractPropFields(
  propsSchema: Record<string, unknown> | null | undefined,
  props: Record<string, unknown> | null | undefined,
): PropFieldDescriptor[] {
  const properties =
    (propsSchema?.properties as Record<string, Record<string, unknown>> | undefined) ?? {};
  const safeProps = props ?? {};
  const keys = Array.from(new Set([...Object.keys(properties), ...Object.keys(safeProps)]));

  return keys.map((key) => {
    const schemaDef = properties[key] ?? {};
    const rawVal = props?.[key] ?? schemaDef.default;
    const schemaType = String(
      schemaDef.type ?? (rawVal !== undefined && rawVal !== null ? typeof rawVal : "string"),
    );

    let kind: PropFieldKind = "string";
    if (schemaType === "number" || schemaType === "integer") {
      kind = "number";
    } else if (schemaType === "boolean") {
      kind = "boolean";
    } else if (
      schemaDef.format === "color" ||
      (typeof rawVal === "string" && HEX_COLOR_RE.test(rawVal))
    ) {
      kind = "color";
    }

    let defaultValue: string | number | boolean;
    if (kind === "number") {
      const parsed = Number(rawVal ?? 0);
      defaultValue = Number.isFinite(parsed) ? parsed : 0;
    } else if (kind === "boolean") {
      defaultValue = parseBooleanProp(rawVal);
    } else if (kind === "color") {
      const candidate = typeof rawVal === "string" ? rawVal.trim() : "";
      defaultValue = HEX_COLOR_RE.test(candidate) ? candidate : "#F05A00";
    } else {
      defaultValue = stringifyPropValue(rawVal);
    }

    return {
      key,
      label: key.replace(/[_-]+/g, " "),
      kind,
      value: defaultValue,
    };
  });
}
