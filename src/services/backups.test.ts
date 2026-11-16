import { expect, it } from "vitest";
import { isBackupStatus } from "./backups";

it.each([
  {state:"idle"}, {state:"cancelled",id:1}, {state:"failed",id:1,message:"Fixed error"},
  {state:"complete",id:1,bytes:0,sha256:"a".repeat(64)},
  ...["preparing","checking","hashing","publishing"].map((phase) => ({state:"running",id:1,cancelling:false,progress:{phase}})),
  {state:"running",id:1,cancelling:true,progress:{phase:"copying",copiedPages:0,totalPages:0}},
])("accepts supported backup status %j", (status) => expect(isBackupStatus(status)).toBe(true));

it.each([
  null, undefined, "idle", 1, {}, {state:"unknown",id:1},
  ...[undefined,"1",NaN,Infinity,-1,1.5,0,0x1_0000_0000].map((id) => ({state:"cancelled",id})),
  {state:"failed",id:1,message:9},
  {state:"complete",id:1,bytes:-1,sha256:"a".repeat(64)},
  {state:"complete",id:1,bytes:0,sha256:9},
  {state:"complete",id:1,bytes:0,sha256:"not-a-checksum"},
  {state:"running",id:1,cancelling:"no",progress:{phase:"preparing"}},
  ...[undefined,"copying",{}, {phase:"unknown"}, {phase:"copying",copiedPages:-1,totalPages:0}, {phase:"copying",copiedPages:0,totalPages:-1}, {phase:"copying",copiedPages:5,totalPages:1}, {phase:"copying",copiedPages:0,totalPages:0x1_0000_0000}].map((progress) => ({state:"running",id:1,cancelling:false,progress})),
])("refuses invalid backup status %j", (status) => expect(isBackupStatus(status)).toBe(false));
