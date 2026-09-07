import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

interface LeadProfile {
  username: string;
  name?: string;
  bio?: string;
  company?: string;
  location?: string;
  followers: number;
  publicRepos: number;
  score: number;
  isKOL?: boolean;
  isMutual?: boolean;
  followedAt?: string;
  source: string;
  topRepoStarred?: string;
}

interface GrowthCache {
  followedUsers: Record<string, { followedAt: string; unfollowed?: boolean; score?: number; isMutual?: boolean }>;
  starredRepos: Record<string, string>;
  leads: Record<string, LeadProfile>;
  lastRun: string;
}

const CACHE_DIR = process.env.GROWTH_CACHE_DIR || join(process.cwd(), ".cache");
const CACHE_FILE = join(CACHE_DIR, "growth-cache.json");

const GITHUB_TOKEN = process.env.GROWTH_PAT || process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
const TARGET_USER = process.env.GITHUB_REPOSITORY_OWNER || "SoCkEt7";

const CONFIG = {
  maxFollowsPerRun: 12,
  maxStarsPerRun: 6,
  minDelayMs: 2500,
  maxDelayMs: 6000,
  topicQueries: [
    "topic:cybersecurity stars:>50 pushed:>2026-08-01",
    "topic:ratatui stars:>10 pushed:>2026-08-01",
    "topic:llm topic:agent stars:>100 pushed:>2026-08-01",
    "topic:ebpf stars:>30 pushed:>2026-08-01",
    "language:rust stars:>100 pushed:>2026-08-15",
    "topic:offensive-security stars:>50",
    "topic:tui language:rust stars:>50",
  ],
  keywordsScore: [
    "cto", "founder", "co-founder", "ceo", "security", "cyber", "infosec", 
    "architect", "lead", "staff", "principal", "rust", "tui", "agentic", "ai", "devops", "head of"
  ]
};

function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function randomDelay() {
  const ms = Math.floor(
    Math.random() * (CONFIG.maxDelayMs - CONFIG.minDelayMs + 1) + CONFIG.minDelayMs
  );
  return sleep(ms);
}

function ensureDirectories() {
  mkdirSync(CACHE_DIR, { recursive: true });
}

function loadCache(): GrowthCache {
  ensureDirectories();
  let cache: GrowthCache = { followedUsers: {}, starredRepos: {}, leads: {}, lastRun: new Date().toISOString() };
  if (existsSync(CACHE_FILE)) {
    try {
      cache = { ...cache, ...JSON.parse(readFileSync(CACHE_FILE, "utf8")) };
    } catch {}
  }
  return cache;
}

function saveCache(cache: GrowthCache) {
  ensureDirectories();
  cache.lastRun = new Date().toISOString();
  writeFileSync(CACHE_FILE, JSON.stringify(cache, null, 2), "utf8");
}

async function githubFetch(endpoint: string, options: RequestInit = {}) {
  const url = endpoint.startsWith("http") ? endpoint : `https://api.github.com${endpoint}`;
  const headers = {
    Accept: "application/vnd.github.v3+json",
    "User-Agent": "Sovereign-Growth-Engine",
    ...(GITHUB_TOKEN ? { Authorization: `Bearer ${GITHUB_TOKEN}` } : {}),
    ...options.headers,
  };

  const res = await fetch(url, { ...options, headers });
  const remaining = res.headers.get("x-ratelimit-remaining");
  if (remaining && parseInt(remaining, 10) < 50) {
    console.warn(`[WARN] Low API Rate Limit remaining: ${remaining}. Throttling...`);
    await sleep(10000);
  }
  return res;
}

async function checkTokenCapabilities(): Promise<{ canFollow: boolean; canStar: boolean; scopes: string }> {
  const res = await githubFetch("/user");
  const scopes = res.headers.get("x-oauth-scopes") || "";
  const canFollow = scopes.includes("user:follow") || scopes.includes("user");
  const canStar = scopes.includes("repo") || scopes.includes("public_repo");
  return { canFollow, canStar, scopes };
}

async function getTargetRepositories(): Promise<{ full_name: string; owner: string }[]> {
  const repos: { full_name: string; owner: string }[] = [];
  const selectedQuery = CONFIG.topicQueries[Math.floor(Math.random() * CONFIG.topicQueries.length)];
  console.log(`🔍 Query: "${selectedQuery}"`);

  const res = await githubFetch(`/search/repositories?q=${encodeURIComponent(selectedQuery)}&sort=updated&order=desc&per_page=15`);
  if (!res.ok) return [];
  const data = await res.json();
  for (const item of data.items || []) {
    repos.push({ full_name: item.full_name, owner: item.owner?.login });
  }
  return repos;
}

async function getRecentStargazers(repoFullName: string): Promise<string[]> {
  const res = await githubFetch(`/repos/${repoFullName}/stargazers?per_page=25`, {
    headers: { Accept: "application/vnd.github.v3.star+json" },
  });
  if (!res.ok) return [];
  const data = await res.json();
  if (!Array.isArray(data)) return [];
  return data
    .map((entry: any) => entry.user?.login || entry.login)
    .filter((u: string) => u && u !== TARGET_USER);
}

async function getUserTopRepo(username: string): Promise<string | null> {
  const res = await githubFetch(`/users/${username}/repos?sort=stars&direction=desc&per_page=1`);
  if (!res.ok) return null;
  const data = await res.json();
  if (Array.isArray(data) && data.length > 0 && !data[0].fork) {
    return data[0].full_name;
  }
  return null;
}

async function evaluateUser(username: string): Promise<{ profile: any; score: number; isQualified: boolean; isKOL: boolean }> {
  const res = await githubFetch(`/users/${username}`);
  if (!res.ok) return { profile: null, score: 0, isQualified: false, isKOL: false };
  const user = await res.json();

  if (user.type === "Bot" || user.public_repos === 0 || user.followers === 0) {
    return { profile: user, score: 0, isQualified: false, isKOL: false };
  }

  let score = 5;
  const bioText = `${user.bio || ""} ${user.company || ""} ${user.name || ""}`.toLowerCase();

  for (const keyword of CONFIG.keywordsScore) {
    if (bioText.includes(keyword)) {
      score += 15;
    }
  }

  const isKOL = user.followers >= 500;
  if (isKOL) score += 25;
  else if (user.followers > 100) score += 10;
  else if (user.followers > 20) score += 5;

  if (user.blog) score += 5;
  if (user.twitter_username) score += 5;

  return { profile: user, score, isQualified: score >= 10, isKOL };
}

async function followUser(username: string): Promise<boolean> {
  const res = await githubFetch(`/user/following/${username}`, {
    method: "PUT",
    headers: { "Content-Length": "0" },
  });
  return res.status === 204;
}

async function starRepo(repoFullName: string): Promise<boolean> {
  const res = await githubFetch(`/user/starred/${repoFullName}`, {
    method: "PUT",
    headers: { "Content-Length": "0" },
  });
  return res.status === 204 || res.status === 307;
}

async function run() {
  console.log("⚡ Sovereign Profile Growth & Visibility Engine");
  if (!GITHUB_TOKEN) {
    console.error("❌ Erreur: Token d'authentification manquant.");
    process.exit(1);
  }

  const { canFollow, canStar, scopes } = await checkTokenCapabilities();
  console.log(`🔐 Scopes actifs: [${scopes}]`);

  const cache = loadCache();
  if (!cache.leads) cache.leads = {};

  const repos = await getTargetRepositories();
  let followCount = 0;
  let starCount = 0;
  let qualifiedCount = 0;

  for (const repo of repos) {
    if (starCount >= CONFIG.maxStarsPerRun && followCount >= CONFIG.maxFollowsPerRun) break;

    const candidateUsers = new Set<string>();
    if (repo.owner && repo.owner !== TARGET_USER) candidateUsers.add(repo.owner);

    const stargazers = await getRecentStargazers(repo.full_name);
    for (const u of stargazers) candidateUsers.add(u);

    for (const username of candidateUsers) {
      if (cache.followedUsers[username] || cache.leads[username]) continue;

      const { profile, score, isQualified, isKOL } = await evaluateUser(username);
      if (!isQualified || !profile) continue;

      qualifiedCount++;
      const kolTag = isKOL ? " [KOL]" : "";
      console.log(`🎯 Lead: @${username}${kolTag} (Score: ${score}) | ${profile.name || username}`);

      let topRepoStarred: string | undefined = undefined;
      if (starCount < CONFIG.maxStarsPerRun && canStar) {
        const topRepo = await getUserTopRepo(username);
        if (topRepo && !cache.starredRepos[topRepo]) {
          const starred = await starRepo(topRepo);
          if (starred) {
            cache.starredRepos[topRepo] = new Date().toISOString();
            topRepoStarred = topRepo;
            starCount++;
            console.log(`  ⭐ Starred: ${topRepo}`);
            await randomDelay();
          }
        }
      }

      let followedAt: string | undefined = undefined;
      if (canFollow && followCount < CONFIG.maxFollowsPerRun) {
        const followed = await followUser(username);
        if (followed) {
          followedAt = new Date().toISOString();
          cache.followedUsers[username] = { followedAt, score, isMutual: false };
          followCount++;
          console.log(`  ✅ Followed: @${username}`);
          await randomDelay();
        }
      }

      cache.leads[username] = {
        username,
        name: profile.name,
        bio: profile.bio,
        company: profile.company,
        location: profile.location,
        followers: profile.followers,
        publicRepos: profile.public_repos,
        score,
        isKOL,
        followedAt,
        source: repo.full_name,
        topRepoStarred,
      };
    }
  }

  saveCache(cache);
  console.log(`\n🏆 Exécution terminée : +${qualifiedCount} leads qualifiés, +${starCount} stars, +${followCount} follows.`);
}

run().catch((err) => {
  console.error("💥 Erreur:", err);
  process.exit(1);
});
