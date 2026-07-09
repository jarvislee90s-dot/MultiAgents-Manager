import { PERMISSION_RISK, PERMISSION_DESCRIPTION } from "@/lib/schemas/manifest";

const RISK_STYLES: Record<string, string> = {
  low: "bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300",
  medium: "bg-yellow-100 text-yellow-700 dark:bg-yellow-900 dark:text-yellow-300",
  high: "bg-red-100 text-red-700 dark:bg-red-900 dark:text-red-300",
};

export function PermissionBadge({ permission }: { permission: string }) {
  const risk = PERMISSION_RISK[permission] ?? "low";
  const desc = PERMISSION_DESCRIPTION[permission] ?? permission;
  return (
    <span className={`inline-flex items-center rounded px-2 py-0.5 text-xs font-medium ${RISK_STYLES[risk]}`} title={desc}>
      {permission}
    </span>
  );
}
