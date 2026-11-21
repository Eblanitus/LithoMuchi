package dev.litospherity.fabric;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.densityfunction.DensityFunction;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.NoiseRouter;

/**
 * Ф4/Ф5: проверка на headless-сервере в CI ({@code -Dlitospherity.ci=<режим>}).
 * Ф4: поля роутера мира в 1000 колонках равны ядру (сид, режим и проводка датапака верны).
 * Ф5: время полной генерации области чанков. Отчёт — {@code litospherity-ci.txt}, затем сервер выключается.
 */
final class LithoCi {
    private LithoCi() {}

    static void register() {
        String mode = System.getProperty("litospherity.ci");
        if (mode == null) return;
        ServerLifecycleEvents.SERVER_STARTED.register(server -> run(server, mode));
    }

    private static void run(MinecraftServer server, String mode) {
        if (mode.startsWith("water-live-")) { LithoWaterFeature.LIVE_STEPS = true; LithoCiWater.run(server, mode.substring("water-live-".length())); return; }
        if (mode.startsWith("water-")) {
            LithoCiWater.run(server, mode.substring("water-".length()));
            return;
        }
        List<String> report = new ArrayList<>();
        boolean ok = true;
        try {
            ServerLevel level = server.overworld();
            long seed = level.getSeed();
            int m = mode.equals("scientific") ? 0 : 1;
            report.add("mode=" + mode + " seed=" + seed + " height=" + level.getHeight());
            if ((m == 0) != (level.getHeight() >= 512)) { ok = false; report.add("FAIL: высота мира не та для режима"); }
            RandomState rs = level.getChunkSource().randomState();
            if (!(level.getChunkSource().getGenerator() instanceof NoiseBasedChunkGenerator noise))
                throw new IllegalStateException("генератор мира не шумовой");
            NoiseRouter r = noise.generatorSettings().value().noiseRouter();
            DensityFunction[] fns = {r.continents(), r.erosion(), r.ridges(), r.temperature(), r.vegetation()};
            String[] names = {"continents", "erosion", "ridges", "temperature", "vegetation"};
            LithoNative.Generator gen = LithoFields.generator(LithoFields.binding(rs));
            int bad = 0;
            for (int i = 0; i < 1000; i++) {
                int x = (int) Math.floorMod(i * 7919L, 40000) - 20000;
                int z = (int) Math.floorMod(i * 104729L, 40000) - 20000;
                for (int f = 0; f < fns.length; f++) {
                    double got = rs.sampleBlockValueUncached(fns[f], x, 64, z);
                    double want = gen.field(f, x, z);
                    if (Math.abs(got - want) > 1e-6) {
                        if (bad++ < 20) report.add("FAIL " + names[f] + " (" + x + "," + z + "): мир " + got + " ядро " + want);
                    }
                }
            }
            report.add("parity: расхождений " + bad + " из 5000");
            // Parity of the running preset is checked above. W can legitimately
            // differ from R1; the frozen R2 contract must compare R2 itself.
            if (LithoFields.binding(rs).version() != LithoFields.VERSION_W) {
                ok = false;
                report.add("FAIL: пресет Litospherity не использует v1-dev-w");
            }
            LithoNative.Generator dev = LithoFields.generator(
                    new LithoFields.Binding(seed, m, LithoFields.VERSION_V1_DEV));
            LithoNative.Generator r2 = LithoFields.generator(
                    new LithoFields.Binding(seed, m, LithoFields.VERSION_R2));
            int badCross = 0;
            for (int i = 0; i < 1000; i++) {
                int x = (int) Math.floorMod(i * 7919L, 40000) - 20000;
                int z = (int) Math.floorMod(i * 104729L, 40000) - 20000;
                for (int f = 0; f < fns.length; f++) {
                    if (Float.floatToRawIntBits((float) r2.field(f, x, z))
                            != Float.floatToRawIntBits((float) dev.field(f, x, z))) badCross++;
                }
            }
            report.add("Д-127: R2 против v1-dev: " + badCross + " / 5000");
            if (bad > 0 || badCross > 0) ok = false;

            int radius = Integer.getInteger("litospherity.ci.radius", 8);
            long t0 = System.nanoTime();
            int n = 0;
            for (int cx = -radius; cx < radius; cx++)
                for (int cz = -radius; cz < radius; cz++) { level.getChunk(cx + 300, cz + 300, ChunkStatus.FULL, true); n++; }
            double ms = (System.nanoTime() - t0) / 1e6;
            report.add(String.format("speed: %d чанков за %.0f мс, %.1f мс/чанк", n, ms, ms / n));
        } catch (Throwable t) {
            ok = false;
            report.add("FAIL: " + t);
            LithoMod.LOG.error("Litospherity CI", t);
        }
        report.add(ok ? "RESULT: PASS" : "RESULT: FAIL");
        report.forEach(l -> LithoMod.LOG.info("Litospherity CI: {}", l));
        try { Files.write(Path.of("litospherity-ci.txt"), report); } catch (Exception ignored) {}
        server.halt(false);
    }
}
