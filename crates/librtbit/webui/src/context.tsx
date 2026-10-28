import { createContext } from "react";
import {
  AltSpeedConfig,
  AltSpeedSchedule,
  AltSpeedStatus,
  BrowseResponse,
  CategoryInfo,
  DhtStats,
  AdvancedConfig,
  FoldersConfig,
  DownloadsConfig,
  NetworkConfig,
  LimitsConfig,
  RtbitAPI,
  SeedLimits,
  SeedLimitsConfig,
  SessionStats,
  TorrentLimits,
} from "./api-types";

export const APIContext = createContext<RtbitAPI>({
  listTorrents: () => {
    throw new Error("Function not implemented.");
  },
  getTorrentDetails: () => {
    throw new Error("Function not implemented.");
  },
  getTorrentStats: () => {
    throw new Error("Function not implemented.");
  },
  getTrackerStatus: () => {
    throw new Error("Function not implemented.");
  },
  getWebSeeds: () => {
    throw new Error("Function not implemented.");
  },
  getPeerStats: () => {
    throw new Error("Function not implemented.");
  },
  uploadTorrent: () => {
    throw new Error("Function not implemented.");
  },
  updateOnlyFiles: () => {
    throw new Error("Function not implemented.");
  },
  pause: () => {
    throw new Error("Function not implemented.");
  },
  start: () => {
    throw new Error("Function not implemented.");
  },
  recheck: () => {
    throw new Error("Function not implemented.");
  },
  addTrackers: () => {
    throw new Error("Function not implemented.");
  },
  forget: () => {
    throw new Error("Function not implemented.");
  },
  delete: () => {
    throw new Error("Function not implemented.");
  },
  getTorrentStreamUrl: () => {
    throw new Error("Function not implemented.");
  },
  getStreamLogsUrl: function (): string | null {
    throw new Error("Function not implemented.");
  },
  getPlaylistUrl: function (index: number): string | null {
    throw new Error("Function not implemented.");
  },
  stats: function (): Promise<SessionStats> {
    throw new Error("Function not implemented.");
  },
  getTorrentHaves: function (index: number): Promise<Uint8Array> {
    throw new Error("Function not implemented.");
  },
  getLimits: function (): Promise<LimitsConfig> {
    throw new Error("Function not implemented.");
  },
  setLimits: function (limits: LimitsConfig): Promise<void> {
    throw new Error("Function not implemented.");
  },
  getDhtStats: function (): Promise<DhtStats> {
    throw new Error("Function not implemented.");
  },
  setRustLog: function (value: string): Promise<void> {
    throw new Error("Function not implemented.");
  },
  getMetadata: function (index: number): Promise<Uint8Array> {
    throw new Error("Function not implemented.");
  },
  getCategories: function (): Promise<Record<string, CategoryInfo>> {
    throw new Error("Function not implemented.");
  },
  createCategory: function (name: string, savePath?: string): Promise<void> {
    throw new Error("Function not implemented.");
  },
  deleteCategory: function (name: string): Promise<void> {
    throw new Error("Function not implemented.");
  },
  setTorrentCategory: function (
    torrentId: number,
    category: string | null,
  ): Promise<void> {
    throw new Error("Function not implemented.");
  },
  getTags: function (): Promise<string[]> {
    throw new Error("Function not implemented.");
  },
  createTags: function (_tags: string[]): Promise<void> {
    throw new Error("Function not implemented.");
  },
  deleteTags: function (_tags: string[]): Promise<void> {
    throw new Error("Function not implemented.");
  },
  addTorrentTags: function (_ids: number[], _tags: string[]): Promise<void> {
    throw new Error("Function not implemented.");
  },
  removeTorrentTags: function (
    _ids: number[],
    _tags: string[],
  ): Promise<void> {
    throw new Error("Function not implemented.");
  },
  getAltSpeed: function (): Promise<AltSpeedStatus> {
    throw new Error("Function not implemented.");
  },
  toggleAltSpeed: function (_enabled: boolean): Promise<void> {
    throw new Error("Function not implemented.");
  },
  setAltSpeedConfig: function (_config: AltSpeedConfig): Promise<void> {
    throw new Error("Function not implemented.");
  },
  getSpeedSchedule: function (): Promise<AltSpeedSchedule> {
    throw new Error("Function not implemented.");
  },
  setSpeedSchedule: function (_schedule: AltSpeedSchedule): Promise<void> {
    throw new Error("Function not implemented.");
  },
  getSeedLimits: function (): Promise<SeedLimitsConfig> {
    throw new Error("Function not implemented.");
  },
  setSeedLimits: function (_limits: SeedLimitsConfig): Promise<void> {
    throw new Error("Function not implemented.");
  },
  getAdvanced: function (): Promise<AdvancedConfig> {
    throw new Error("Function not implemented.");
  },
  setAdvanced: function (_config: AdvancedConfig): Promise<void> {
    throw new Error("Function not implemented.");
  },
  setTorrentSeedLimits: function (
    _id: number,
    _limits: SeedLimits,
  ): Promise<void> {
    throw new Error("Function not implemented.");
  },
  getTorrentLimits: function (_id: number): Promise<TorrentLimits> {
    throw new Error("Function not implemented.");
  },
  setTorrentLimits: function (
    _id: number,
    _limits: TorrentLimits,
  ): Promise<void> {
    throw new Error("Function not implemented.");
  },
  setSequential: function (_id: number, _enabled: boolean): Promise<void> {
    throw new Error("Function not implemented.");
  },
  setSuperSeed: function (_id: number, _enabled: boolean): Promise<void> {
    throw new Error("Function not implemented.");
  },
  queueMoveTop: function (_id: number): Promise<void> {
    throw new Error("Function not implemented.");
  },
  queueMoveBottom: function (_id: number): Promise<void> {
    throw new Error("Function not implemented.");
  },
  queueMoveUp: function (_id: number): Promise<void> {
    throw new Error("Function not implemented.");
  },
  queueMoveDown: function (_id: number): Promise<void> {
    throw new Error("Function not implemented.");
  },
  getFolders: function (): Promise<FoldersConfig> {
    throw new Error("Function not implemented.");
  },
  setFolders: function (_config: FoldersConfig): Promise<void> {
    throw new Error("Function not implemented.");
  },
  getDownloadsConfig: function (): Promise<DownloadsConfig> {
    throw new Error("Function not implemented.");
  },
  setDownloadsConfig: function (_config: DownloadsConfig): Promise<void> {
    throw new Error("Function not implemented.");
  },
  getNetworkConfig: function (): Promise<NetworkConfig> {
    throw new Error("Function not implemented.");
  },
  browseDirectory: function (_path?: string): Promise<BrowseResponse> {
    throw new Error("Function not implemented.");
  },
});
