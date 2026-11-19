package dev.litospherity.fabric;

import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLevelEvents;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;

/**
 * Ф6-В1 (PLAN-WATER-FABRIC §5, решение 8.2): версия генератора — по пресету мира.
 *
 * <p>Пресеты {@code litospherity:gameplay} и {@code litospherity:scientific} — это {@code v1-dev-r2}
 * (вода выше уровня моря). Любой другой мир («Обычный» с модом) остаётся на {@code v1-dev}: там нет
 * нашего пресета, и вода выше моря не появляется.
 */
final class LithoWorlds {
    private LithoWorlds() {}

    static void register() {
        ServerLevelEvents.LOAD.register(LithoWorlds::loaded);
    }

    private static void loaded(net.minecraft.server.MinecraftServer server, ServerLevel level) {
        try {
            if (!(level.getChunkSource().getGenerator() instanceof NoiseBasedChunkGenerator noise)) return;
            String path = noise.generatorSettings().unwrapKey()
                    .map(key -> key.identifier().getPath())
                    .orElse("");
            boolean ours = noise.generatorSettings().unwrapKey()
                    .map(key -> key.identifier().getNamespace().equals(LithoMod.MOD_ID))
                    .orElse(false);
            int version = ours ? LithoFields.VERSION_R2 : LithoFields.VERSION_V1_DEV;
            int mode = level.getHeight() >= 512 ? 0 : 1;
            LithoFields.VERSIONS.put(level.getChunkSource().randomState(), version);
            LithoFields.MODES.put(level.getChunkSource().randomState(), mode);
            if (level.dimension().equals(net.minecraft.world.level.Level.OVERWORLD)) {
                LithoFields.setWorld(new LithoFields.Binding(level.getSeed(), mode, version));
            }
            LithoMod.LOG.info("Litospherity: мир {} — пресет «{}», версия {} режим {}",
                    level.dimension().location(), path,
                    version == LithoFields.VERSION_R2 ? "v1-dev-r2" : "v1-dev",
                    mode == 0 ? "scientific" : "gameplay");
        } catch (Throwable error) {
            LithoMod.LOG.warn("Litospherity: не удалось определить пресет мира", error);
        }
    }
}
