import { JetsonDevice } from "../features/connection/types";

export function DeviceCard({ device }: { device: JetsonDevice }) {
  const details = [
    device.model,
    device.jetpackVersion && `JetPack ${device.jetpackVersion}`,
    device.ubuntuVersion && `Ubuntu ${device.ubuntuVersion}`,
    device.l4tVersion && `L4T ${device.l4tVersion}`,
    device.architecture,
  ].filter((v): v is string => Boolean(v));

  return (
    <div className="rounded-lg border border-zinc-200 bg-white px-4 py-3 text-left dark:border-zinc-700 dark:bg-zinc-800">
      <div className="text-sm font-medium text-zinc-900 dark:text-zinc-100">
        {device.hostname ?? device.host}
      </div>
      {details.length > 0 && (
        <div className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">
          {details.join(" · ")}
        </div>
      )}
    </div>
  );
}