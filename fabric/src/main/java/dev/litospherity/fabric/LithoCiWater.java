package dev.litospherity.fabric;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerTickEvents;
import net.minecraft.core.BlockPos;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.status.ChunkStatus;

/**
 * Ф6-В5 (PLAN-WATER-FABRIC §5): CI «вода не стекает по склону».
 *
 * <p>Режим {@code -Dlitospherity.ci=water-<режим>}. Читает {@code run/water-points.txt}
 * (вывод {@code litho-sandbox water-points}), генерирует вокруг каждого места 5×5 чанков,
 * держит их загруженными 400 тиков и считает метрики. Провал job'а — {@code RESULT: FAIL}.
 */
final class LithoCiWater {
    private LithoCiWater() {}

    /** Сколько тиков ждать, чтобы тики жидкости прошли (план: 400). */
    private static final int TICKS = 400;
    /** Радиус области вокруг места, чанки: 5×5. */
    private static final int RADIUS = 2;

    /** Сгенерированные чанки окна: пары (x, z) в одной long. */
    private static final Set<Long> CHUNKS = new LinkedHashSet<>();

    static void run(MinecraftServer server, String mode) {
        LithoWaterFeature.STEPS.set(0);
        List<String> report = new ArrayList<>();
        try {
            ServerLevel level = server.overworld();
            List<int[]> points = readPoints();
            report.add("water: режим " + mode + " сид " + level.getSeed() + " мест " + points.size());
            if (points.isEmpty()) {
                report.add("FAIL: нет файла run/water-points.txt или в нём нет мест");
                finish(server, report, false);
                return;
            }
            LithoNative.Generator gen = LithoFields.generator(LithoFields.binding(level.getChunkSource().randomState()));
            long t0 = System.nanoTime();
            Set<Long> generated = CHUNKS;
            generated.clear();
            for (int[] p : points) {
                int cx = p[0] >> 4;
                int cz = p[1] >> 4;
                for (int dx = -RADIUS; dx <= RADIUS; dx++) {
                    for (int dz = -RADIUS; dz <= RADIUS; dz++) {
                        level.getChunk(cx + dx, cz + dz, ChunkStatus.FULL, true);
                        if (generated.add((((long) (cx + dx)) << 32) ^ (cz + dz))) {
                            level.setChunkForced(cx + dx, cz + dz, true);
                        }
                    }
                }
            }
            double ms = (System.nanoTime() - t0) / 1e6;
            report.add(String.format("вода: сгенерировано %d чанков за %.0f мс, %.1f мс/чанк",
                    generated.size(), ms, ms / Math.max(1, generated.size())));
            long sealsAtStart = LithoWaterFeature.SEALS.get();
            long chunksWithWater = LithoWaterFeature.CHUNKS.get();
            long sourcesBefore = countWater(level, true);
            long flowingBefore = countWater(level, false);
            // Ждём 400 тиков, чтобы обновления жидкости прошли.
            long[] tickStart = {0L}; long[] totalTickNanos = {0L}; long[] maxTickNanos = {0L}; int[] measuredTicks = {0}; boolean[] done = {false}; ServerTickEvents.START_SERVER_TICK.register(s -> { if (!done[0]) tickStart[0] = System.nanoTime(); });
            int[] left = {TICKS};
            ServerTickEvents.END_SERVER_TICK.register(s -> {
                if ( (done[0]) return; if (tickStart[0] != 0L) { long dt = System.nanoTime() - tickStart[0]; totalTickNanos[0] += dt; if (dt > maxTickNanos[0]) maxTickNanos[0] = dt; measuredTicks[0]++; }
                if (left[0]-- > 0) return; done[0] = true; double avgTickMs = measuredTicks[0] == 0 ? 0.0 : (totalTickNanos[0] / (double) measuredTicks[0]) / 1_000_000.0; double maxTickMs = maxTickNanos[0] / 1_000_000.0;
                measure(server, level, gen, points, report, sealsAtStart, chunksWithWater, sourcesBefore, flowingBefore, ms / Math.max(1, generated.size()), avgTickMs, maxTickMs);
            });
        } catch (Throwable t) {
            report.add("FAIL: " + t);
            LithoMod.LOG.error("Litospherity CI water", t);
            finish(server, report, false);
        }
    }

    private static void measure(MinecraftServer server, ServerLevel level, LithoNative.Generator gen,
                                List<int[]> points, List<String> report, long sealsAtStart, long chunksWithWater,
                                long sourcesBefore, long flowingBefore, double msPerChunk, double avgTickMs, double maxTickMs) {
        boolean ok = true;
        try {
            long flowingAfter = countWater(level, false);
            long sourcesAfter = countWater(level, true);
            long growth = flowingAfter - flowingBefore;
            double growthPct = sourcesAfter == 0 ? 0.0 : 100.0 * growth / (double) sourcesAfter;
            long seals = LithoWaterFeature.SEALS.get() - sealsAtStart;
            double sealsPerChunk = chunksWithWater == 0 ? 0.0 : seals / (double) chunksWithWater;

            long leaks = 0;
            long wet = 0;
            long dry = 0;
            for (int[] p : points) {
                for (int x = (p[0] & ~15) - RADIUS * 16; x < (p[0] & ~15) + (RADIUS + 1) * 16; x++) {
                    for (int z = (p[1] & ~15) - RADIUS * 16; z < (p[1] & ~15) + (RADIUS + 1) * 16; z++) {
                        LithoNative.WaterColumns core = gen.water(new double[] {x}, new double[] {z});
                        int top = core.level()[0];
                        if (top == LithoNative.WaterColumns.NONE || top <= LithoWaterFeature.SEA_LEVEL) continue;
                        int y = top - 1;
                        BlockState state = level.getBlockState(new BlockPos(x, y, z));
                        if (state.is(Blocks.WATER)) {
                            wet++;
                        } else {
                            dry++;
                        }
                        // Протечка: вода в колонке, где у ядра воды на этой высоте нет.
                        for (int yy = Math.max(LithoWaterFeature.SEA_LEVEL + 1, y - 2); yy <= y + 1; yy++) {
                            if (!level.getBlockState(new BlockPos(x, yy, z)).is(Blocks.WATER)) continue;
                            boolean coreHere = false;
                            for (int dx = -1; dx <= 1 && !coreHere; dx++) {
                                for (int dz = -1; dz <= 1 && !coreHere; dz++) {
                                    LithoNative.WaterColumns near = gen.water(new double[] {x + dx}, new double[] {z + dz});
                                    coreHere = near.level()[0] != LithoNative.WaterColumns.NONE
                                            && near.level()[0] > yy;
                                }
                            }
                            if (!coreHere) leaks++;
                        }
                    }
                }
            }
            double dryPct = wet + dry == 0 ? 0.0 : 100.0 * dry / (double) (wet + dry);
            report.add(String.format("протечки: %d (порог 0)", leaks));
            report.add(String.format("сухое русло: %d из %d, %.2f %% (порог 1 %%)", dry, wet + dry, dryPct));
            report.add(String.format("рост текущей воды за %d тиков: %d, %.2f %% источников (порог 5 %%)",
                    TICKS, growth, growthPct));
            report.add(String.format("заделки герметизации: %d на %d чанков, %.2f на чанк (порог 2)",
                    seals, chunksWithWater, sealsPerChunk)); report.add(String.format(java.util.Locale.ROOT, "steps: %d, liveSteps=%s, avgTick=%.2f ms, maxTick=%.2f ms", LithoWaterFeature.STEPS.get(), LithoWaterFeature.LIVE_STEPS, avgTickMs, maxTickMs));
            report.add(String.format("время генерации: %.1f мс/чанк (замер Ф5 — в отчёте job'а)", msPerChunk));
            report.add(String.format("вода выше моря: колонок с водой %d, источников %d, текущей %d",
                    wet, sourcesAfter, flowingAfter));
            if (leaks > 0) ok = false;
            if (dryPct > 1.0) ok = false;
            if (growthPct > 5.0) ok = false;
            if (sealsPerChunk > 2.0) ok = false;
        } catch (Throwable t) {
            ok = false;
            report.add("FAIL: " + t);
            LithoMod.LOG.error("Litospherity CI water measure", t);
        }
        finish(server, report, ok);
    }

    /** Вода выше моря в загруженных чанках: источники или текущая. */
    private static long countWater(ServerLevel level, boolean sources) {
        long count = 0;
        for (net.minecraft.world.level.chunk.LevelChunk chunk : loadedChunks(level)) {
            for (int x = 0; x < 16; x++) {
                for (int z = 0; z < 16; z++) {
                    for (int y = LithoWaterFeature.SEA_LEVEL + 1; y < level.getMaxY(); y++) {
                        BlockState state = chunk.getBlockState(new BlockPos(
                                chunk.getPos().getMinBlockX() + x, y, chunk.getPos().getMinBlockZ() + z));
                        if (!state.is(Blocks.WATER)) continue;
                        boolean source = state.getFluidState().isSource();
                        if (source == sources) count++;
                    }
                }
            }
        }
        return count;
    }

    /** 26.3: {@code getLoadedChunks()} убран — берём чанки окна по {@code getChunkNow}. */
    private static List<net.minecraft.world.level.chunk.LevelChunk> loadedChunks(ServerLevel level) {
        List<net.minecraft.world.level.chunk.LevelChunk> out = new ArrayList<>();
        for (long key : CHUNKS) {
            net.minecraft.world.level.chunk.LevelChunk chunk =
                    level.getChunkSource().getChunkNow((int) (key >> 32), (int) key);
            if (chunk != null) out.add(chunk);
        }
        return out;
    }

    /** Места из {@code run/water-points.txt}: строки вида {@code x=12 z=-34 level=80}. */
    private static List<int[]> readPoints() {
        List<int[]> points = new ArrayList<>();
        Path file = Path.of("water-points.txt");
        if (!Files.exists(file)) return points;
        try {
            for (String line : Files.readAllLines(file)) {
                String trimmed = line.trim();
                if (!trimmed.startsWith("x=")) continue;
                String[] parts = trimmed.split("\\s+");
                int x = Integer.MIN_VALUE;
                int z = Integer.MIN_VALUE;
                for (String part : parts) {
                    if (part.startsWith("x=")) x = Integer.parseInt(part.substring(2));
                    if (part.startsWith("z=")) z = Integer.parseInt(part.substring(2));
                }
                if (x != Integer.MIN_VALUE && z != Integer.MIN_VALUE) points.add(new int[] {x, z});
                if (points.size() >= 20) break;
            }
        } catch (Exception error) {
            LithoMod.LOG.warn("Litospherity CI: не читаю water-points.txt", error);
        }
        return points;
    }

    private static void finish(MinecraftServer server, List<String> report, boolean ok) {
        report.add(ok ? "RESULT: PASS" : "RESULT: FAIL");
        report.forEach(line -> LithoMod.LOG.info("Litospherity CI water: {}", line));
        try {
            Files.write(Path.of("litospherity-ci.txt"), report);
        } catch (Exception ignored) {
            // отчёт не критичен для выхода сервера
        }
        server.halt(false);
    }
}
