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
  maxFollowsPerRun: 10,
  maxStarsPerRun: 5,
  minDelayMs: 3000,
  maxDelayMs: 7500,
  topicQueries: [
    "topic:cybersecurity stars:>30 pushed:>2026-08-01",
    "topic:ratatui stars:>5 pushed:>2026-08-01",
    "topic:terminal-app language:rust stars:>20",
    "topic:llm-security stars:>20 pushed:>2026-08-01",
    "topic:offensive-security stars:>30 pushed:>2026-08-01",
    "topic:ebpf language:rust stars:>20",
    "topic:zero-trust stars:>20 pushed:>2026-08-01",
    "topic:model-context-protocol stars:>15 pushed:>2026-08-01",
    "topic:agentic-ai stars:>50 pushed:>2026-08-01",
  ],
  userDirectQueries: [
    "location:Paris followers:>20 repos:>5 language:rust",
    "location:France bio:CTO followers:>30",
    "bio:security bio:architect followers:>50",
    "bio:founder language:rust followers:>30",
  ],
  keywordsScore: [
    { word: "cto", weight: 20 },
    { word: "founder", weight: 20 },
    { word: "co-founder", weight: 20 },
    { word: "ceo", weight: 15 },
    { word: "head of security", weight: 25 },
    { word: "security architect", weight: 25 },
    { word: "ciso", weight: 25 },
    { word: "cybersecurity", weight: 15 },
    { word: "infosec", weight: 15 },
    { word: "offensive", weight: 15 },
    { word: "ratatui", weight: 20 },
    { word: "rust", weight: 15 },
    { word: "zero-trust", weight: 15 },
    { word: "nis2", weight: 20 },
    { word: "ebpf", weight: 15 },
    { word: "staff engineer", weight: 15 },
    { word: "principal engineer", weight: 15 },
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

async function getDirectUsers(): Promise<string[]> {
  const selectedQuery = CONFIG.userDirectQueries[Math.floor(Math.random() * CONFIG.userDirectQueries.length)];
  console.log(`🎯 Direct User Search: "${selectedQuery}"`);
  const res = await githubFetch(`/search/users?q=${encodeURIComponent(selectedQuery)}&sort=followers&order=desc&per_page=12`);
  if (!res.ok) return [];
  const data = await res.json();
  return (data.items || []).map((u: any) => u.login).filter((u: string) => u && u !== TARGET_USER);
}

async function checkMutual(username: string): Promise<boolean> {
  const res = await githubFetch(`/users/${username}/following/${TARGET_USER}`);
  return res.status === 204;
}

async function evaluateUser(username: string): Promise<{ profile: any; score: number; isQualified: boolean; isKOL: boolean }> {
  const res = await githubFetch(`/users/${username}`);
  if (!res.ok) return { profile: null, score: 0, isQualified: false, isKOL: false };
  const user = await res.json();

  if (user.type === "Bot" || user.public_repos === 0 || user.followers === 0) {
    return { profile: user, score: 0, isQualified: false, isKOL: false };
  }

  let score = 5;
  const bioText = `${user.bio || ""} ${user.company || ""} ${user.name || ""} ${user.location || ""}`.toLowerCase();

  for (const { word, weight } of CONFIG.keywordsScore) {
    if (bioText.includes(word)) {
      score += weight;
    }
  }

  const isKOL = user.followers >= 400;
  if (isKOL) score += 20;
  else if (user.followers > 80) score += 10;
  else if (user.followers > 20) score += 5;

  if (user.blog) score += 5;
  if (user.twitter_username) score += 5;

  return { profile: user, score, isQualified: score >= 15, isKOL };
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
  console.log("⚡ Sovereign Intelligence & Network Expansion Engine");
  if (!GITHUB_TOKEN) {
    console.warn("⚠️ Token d'authentification non configuré. Mode audit simulé.");
    return;
  }

  const { canFollow, canStar, scopes } = await checkTokenCapabilities();
  console.log(`🔐 Scopes actifs: [${scopes || "lecture seule"}]`);

  const cache = loadCache();
  if (!cache.leads) cache.leads = {};

  // Check recent follows for mutual follow-back
  const recentFollows = Object.keys(cache.followedUsers).slice(-15);
  for (const u of recentFollows) {
    if (cache.followedUsers[u] && !cache.followedUsers[u].isMutual) {
      const isMutual = await checkMutual(u);
      if (isMutual) {
        cache.followedUsers[u].isMutual = true;
        if (cache.leads[u]) cache.leads[u].isMutual = true;
        console.log(`🤝 Mutual Follow Back detected from @${u}!`);
      }
    }
  }

  const candidateUsers = new Set<string>();

  // 1. Direct targeted user search
  const directUsers = await getDirectUsers();
  for (const u of directUsers) candidateUsers.add(u);

  // 2. Targeted repos scan & stargazers
  const repos = await getTargetRepositories();
  for (const repo of repos) {
    if (repo.owner && repo.owner !== TARGET_USER) candidateUsers.add(repo.owner);
    const stargazers = await getRecentStargazers(repo.full_name);
    for (const u of stargazers) candidateUsers.add(u);
  }

  let followCount = 0;
  let starCount = 0;
  let qualifiedCount = 0;

  for (const username of candidateUsers) {
    if (starCount >= CONFIG.maxStarsPerRun && followCount >= CONFIG.maxFollowsPerRun) break;
    if (cache.followedUsers[username] || cache.leads[username]) continue;

    const { profile, score, isQualified, isKOL } = await evaluateUser(username);
    if (!isQualified || !profile) continue;

    qualifiedCount++;
    const kolTag = isKOL ? " [KOL]" : "";
    console.log(`🎯 Lead: @${username}${kolTag} (Score: ${score}) | ${profile.name || username} (${profile.company || profile.location || "N/A"})`);

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
      source: profile.location || "targeted_search",
      topRepoStarred,
    };
  }

  saveCache(cache);
  console.log(`\n🏆 Cycle terminé : +${qualifiedCount} leads qualifiés, +${starCount} stars ciblées, +${followCount} follows.`);
}

run().catch((err) => {
  console.error("💥 Erreur:", err);
  process.exit(1);
});
